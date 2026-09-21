//! Lancer une application ou une commande.

use serde_json::{json, Value};
use std::ptr::null;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

const SORTIE_MAX: usize = 8000;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Ouvre un exécutable, un document, un dossier ou une adresse, comme le ferait
/// « Exécuter » dans Windows : `notepad`, `C:\\docs\\a.pdf`, `https://…`.
pub fn launch(target: &str, args: Option<&str>) -> Result<Value, String> {
    let verb = wide("open");
    let file = wide(target);
    let params = args.map(wide);
    // SAFETY: toutes les chaînes sont terminées par NUL et vivent pendant l'appel.
    let code = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            params.as_ref().map_or(null(), |p| p.as_ptr()),
            null(),
            SW_SHOWNORMAL,
        )
    } as isize;
    if code <= 32 {
        return Err(format!(
            "Windows n'a pas pu ouvrir « {target} » (code {code})"
        ));
    }
    Ok(json!({ "launched": target }))
}

/// Exécute une commande PowerShell et rend sa sortie, tronquée.
pub async fn run(command: &str, cwd: Option<&str>, timeout: Duration) -> Result<Value, String> {
    let script = format!("[Console]::OutputEncoding=[Text.Encoding]::UTF8; {command}");
    let mut cmd = tokio::process::Command::new("powershell");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        &script,
    ])
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::piped())
    .kill_on_drop(true);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("PowerShell introuvable : {e}"))?;
    let mut out = child.stdout.take().ok_or("stdout indisponible")?;
    let mut err = child.stderr.take().ok_or("stderr indisponible")?;
    let read = async {
        let (mut o, mut e) = (Vec::new(), Vec::new());
        let (ro, re) = tokio::join!(out.read_to_end(&mut o), err.read_to_end(&mut e));
        ro.and(re)
            .map_err(|x| format!("lecture de la sortie : {x}"))?;
        let status = child.wait().await.map_err(|x| format!("attente : {x}"))?;
        Ok::<_, String>((o, e, status))
    };
    let (o, e, status) = tokio::time::timeout(timeout, read)
        .await
        .map_err(|_| format!("Commande interrompue après {} s", timeout.as_secs()))??;
    Ok(json!({
        "exit_code": status.code(),
        "stdout": tronque(&String::from_utf8_lossy(&o)),
        "stderr": tronque(&String::from_utf8_lossy(&e)),
    }))
}

fn tronque(s: &str) -> String {
    let t = s.trim();
    if t.chars().count() <= SORTIE_MAX {
        return t.to_string();
    }
    let debut: String = t.chars().take(SORTIE_MAX).collect();
    format!("{debut}\n… (sortie tronquée)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn une_sortie_trop_longue_est_tronquee() {
        let long = "x".repeat(SORTIE_MAX + 50);
        assert!(tronque(&long).ends_with("(sortie tronquée)"));
        assert_eq!(tronque("  court \n"), "court");
    }

    #[tokio::test]
    async fn une_commande_rend_sa_sortie() {
        let r = run("Write-Output 'bonjour é'", None, Duration::from_secs(20))
            .await
            .unwrap();
        assert_eq!(r["stdout"], "bonjour é");
        assert_eq!(r["exit_code"], 0);
    }

    #[tokio::test]
    async fn une_commande_trop_longue_est_interrompue() {
        let e = run("Start-Sleep 30", None, Duration::from_secs(1))
            .await
            .unwrap_err();
        assert!(e.contains("interrompue"), "{e}");
    }
}
