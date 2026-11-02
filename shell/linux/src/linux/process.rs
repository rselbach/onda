//! Bounded desktop helpers. Transcript and credential data travel on stdin.

use std::io::{Read, Write};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

pub fn run(program: &str, args: &[&str], input: &[u8]) -> anyhow::Result<Vec<u8>> {
    run_for(program, args, input, Duration::from_secs(3))
}

pub fn run_for(
    program: &str,
    args: &[&str],
    input: &[u8],
    timeout: Duration,
) -> anyhow::Result<Vec<u8>> {
    let output = execute(program, args, input, timeout, true)?;
    anyhow::ensure!(
        output.status.success(),
        "{program} failed ({})",
        output.status
    );
    Ok(output.stdout)
}

// secret-tool returns exit 1 with no diagnostic when no matching item exists.
pub fn optional(program: &str, args: &[&str], input: &[u8]) -> anyhow::Result<Option<Vec<u8>>> {
    let output = execute(program, args, input, Duration::from_secs(120), true)?;
    if output.status.code() == Some(1) && output.stderr.is_empty() {
        return Ok(None);
    }
    anyhow::ensure!(
        output.status.success(),
        "{program} failed ({}); check that the desktop keyring is available and unlocked",
        output.status
    );
    Ok(Some(output.stdout))
}

// Clipboard owners fork and retain stdout; do not wait for their pipe to close.
pub fn write(program: &str, args: &[&str], input: &[u8]) -> anyhow::Result<()> {
    let output = execute(program, args, input, Duration::from_secs(3), false)?;
    anyhow::ensure!(
        output.status.success(),
        "{program} failed ({})",
        output.status
    );
    Ok(())
}

fn execute(
    program: &str,
    args: &[&str],
    input: &[u8],
    timeout: Duration,
    capture: bool,
) -> anyhow::Result<Output> {
    let deadline = Instant::now() + timeout;
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(if capture {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stderr(if capture {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .spawn()?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("missing helper stdin"))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let input = input.to_vec();
    let (written, writer) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = written.send(stdin.write_all(&input));
    });
    let reader = read_pipe(stdout);
    let errors = read_pipe(stderr);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill()?;
            child.wait()?;
            anyhow::bail!("{program} timed out");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    writer.recv_timeout(deadline.saturating_duration_since(Instant::now()))??;
    let stdout = reader.recv_timeout(deadline.saturating_duration_since(Instant::now()))??;
    let stderr = errors.recv_timeout(deadline.saturating_duration_since(Instant::now()))??;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn read_pipe(
    pipe: Option<impl Read + Send + 'static>,
) -> std::sync::mpsc::Receiver<anyhow::Result<Vec<u8>>> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| {
            let mut data = Vec::new();
            if let Some(pipe) = pipe {
                pipe.take(32 * 1024 * 1024 + 1).read_to_end(&mut data)?;
            }
            anyhow::ensure!(
                data.len() <= 32 * 1024 * 1024,
                "helper output exceeds 32 MiB"
            );
            Ok(data)
        })();
        let _ = sender.send(result);
    });
    receiver
}

pub fn available(program: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|path| {
            std::fs::metadata(path.join(program))
                .is_ok_and(|file| file.is_file() && file.permissions().mode() & 0o111 != 0)
        })
    })
}

pub fn wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some_and(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_secret_is_distinct_from_a_keyring_failure() {
        assert!(optional("sh", &["-c", "exit 1"], &[]).unwrap().is_none());
        let error = optional(
            "sh",
            &["-c", "echo 'Greendale keyring unavailable' >&2; exit 1"],
            &[],
        )
        .unwrap_err();
        assert!(error.to_string().contains("keyring"));
        assert!(
            !error.to_string().contains("Greendale"),
            "helper diagnostics must not expose credential data"
        );
    }

    #[test]
    fn clipboard_owner_can_keep_running_after_its_launcher_exits() {
        write("sh", &["-c", "sleep 4 &"], b"Greendale").unwrap();
    }
}
