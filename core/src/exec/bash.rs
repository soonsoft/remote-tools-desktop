use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellKind {
    Powershell, Pwsh, Cmd, Sh, Custom(String),
}

impl ShellKind {
    pub fn default_for_current_os() -> Self {
        if cfg!(windows) { Self::Powershell } else { Self::Sh }
    }
    fn argv(&self, command: &str) -> Vec<String> {
        match self {
            Self::Powershell => vec!["powershell".into(), "-NoProfile".into(), "-NonInteractive".into(), "-Command".into(), command.into()],
            Self::Pwsh => vec!["pwsh".into(), "-NoProfile".into(), "-NonInteractive".into(), "-Command".into(), command.into()],
            Self::Cmd => vec!["cmd".into(), "/C".into(), command.into()],
            Self::Sh => vec!["sh".into(), "-c".into(), command.into()],
            Self::Custom(exe) => vec![exe.clone(), "-c".into(), command.into()],
        }
    }
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BashOutcome {
    pub stdout: String, pub stderr: String,
    pub exit_code: i32, pub duration_ms: u64, pub killed: bool,
}

pub async fn run_bash(command: &str, timeout: Duration, cwd: &Path, shell: &ShellKind) -> BashOutcome {
    let t0 = Instant::now();
    let mut argv = shell.argv(command);
    let program = argv.remove(0);
    let child = Command::new(program)
        .args(&argv)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => return BashOutcome {
            stdout: String::new(), stderr: format!("无法启动 shell：{e}"),
            exit_code: -1, duration_ms: t0.elapsed().as_millis() as u64, killed: false,
        },
    };
    let mut stdout_pipe = child.stdout.take().unwrap();
    let mut stderr_pipe = child.stderr.take().unwrap();
    let mut out_task = tokio::spawn(async move { let mut b = String::new(); let _ = stdout_pipe.read_to_string(&mut b).await; b });
    let mut err_task = tokio::spawn(async move { let mut b = String::new(); let _ = stderr_pipe.read_to_string(&mut b).await; b });
    let killed;
    let exit_code;
    match tokio::time::timeout(timeout, child.wait()).await {
        Ok(status) => { killed = false; exit_code = status.unwrap().code().unwrap_or(-1); }
        Err(_) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            killed = true; exit_code = -1;
        }
    }
    // 尾部读取加上限：孙进程（如 `cmd /C start /b ...`）会继承管道写端，令 read_to_string 永远等不到 EOF；
    // 宽限期 500ms 后截断排水，保证 run_bash 不会绕过超时契约挂死（killed:true + 截断排水为既定语义）。
    // 以 &mut 借用等待，超时分支才能 abort 排水任务——否则 JoinHandle 被 drop 即 detach，
    // 任务会一直持有读端活到孙进程退出（单线程 runtime 关停还会等它，实测 29s）。
    let stdout = match tokio::time::timeout(Duration::from_millis(500), &mut out_task).await {
        Ok(v) => v.unwrap_or_default(),
        Err(_) => { /* grandchild holds the pipe: truncate drain */ out_task.abort(); String::new() }
    };
    let stderr = match tokio::time::timeout(Duration::from_millis(500), &mut err_task).await {
        Ok(v) => v.unwrap_or_default(),
        Err(_) => { err_task.abort(); String::new() }
    };
    BashOutcome {
        stdout,
        stderr,
        exit_code, duration_ms: t0.elapsed().as_millis() as u64, killed,
    }
}
