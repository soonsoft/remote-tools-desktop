use remote_tools_core::exec::bash::*;
use std::time::Duration;

#[tokio::test]
async fn echo_roundtrip_with_exit_code() {
    let tmp = std::env::temp_dir();
    let out = run_bash("echo RT_BASH_OK_42", Duration::from_secs(10), &tmp, &ShellKind::default_for_current_os()).await;
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.trim().contains("RT_BASH_OK_42"), "stdout: {}", out.stdout);
    assert!(out.duration_ms < 10_000);
}

#[tokio::test]
async fn nonzero_exit_and_stderr_flow_back() {
    let tmp = std::env::temp_dir();
    let shell = ShellKind::default_for_current_os();
    let out = run_bash(if cfg!(windows) {
        "Write-Error 'boom'; exit 3"
    } else {
        "echo boom >&2; exit 3"
    }, Duration::from_secs(10), &tmp, &shell).await;
    assert_eq!(out.exit_code, 3, "stdout={} stderr={}", out.stdout, out.stderr);
    assert!(out.stderr.contains("boom"), "stderr: {}", out.stderr);
}

#[tokio::test]
async fn timeout_kills_and_reports() {
    let tmp = std::env::temp_dir();
    let shell = ShellKind::default_for_current_os();
    let out = run_bash(if cfg!(windows) { "Start-Sleep -Seconds 30" } else { "sleep 30" },
        Duration::from_millis(300), &tmp, &shell).await;
    assert!(out.killed);
    assert!(out.duration_ms < 5_000);
}

#[tokio::test]
async fn cwd_is_honored() {
    let tmp = std::env::temp_dir();
    let shell = ShellKind::default_for_current_os();
    let out = run_bash(if cfg!(windows) { "Get-Location" } else { "pwd" },
        Duration::from_secs(10), &tmp, &shell).await;
    // Windows 的 std::env::temp_dir()（GetTempPathW）带尾部反斜杠，而 Get-Location 输出无尾部分隔符——
    // 归一化后裁掉尾部 '/'，否则 contains 永不匹配（Unix 上为 no-op，语义不变）。
    assert!(out.stdout.to_lowercase().replace('\\', "/").contains(
        tmp.to_string_lossy().to_lowercase().replace('\\', "/").trim_end_matches('/')), "cwd not honored: {}", out.stdout);
}

#[tokio::test]
async fn timeout_returns_even_when_grandchild_holds_pipe() {
    if !cfg!(windows) { return; }
    let tmp = std::env::temp_dir();
    let shell = ShellKind::Cmd;
    let t0 = std::time::Instant::now();
    // ShellKind::Cmd 已前置 `cmd /C`，命令串内不再嵌套。`start /b ping` 是继承 stdout 写端的孙进程；
    // `& ping -n 7 > NUL` 让直接子进程 cmd 存活 ~6s、必然撞上 300ms 超时（否则 cmd 立即退出、killed 恒 false）。
    // 持有进程用 8s/7s 而非 20s/30s：修复后 run_bash ~1.3s 即返回（契约本位），但被遗弃的异步读
    // 句柄要等 tokio runtime 关停排水到孙进程退出才释放——缩短持有时间把该外部效应控制在 ~7s。
    let out = run_bash("start /b ping -n 8 127.0.0.1 & ping -n 7 127.0.0.1 > NUL",
        Duration::from_millis(300), &tmp, &shell).await;
    assert!(out.killed, "killed={} exit_code={} elapsed {:?}", out.killed, out.exit_code, t0.elapsed());
    assert!(t0.elapsed() < Duration::from_secs(5), "elapsed {:?}", t0.elapsed());
}
