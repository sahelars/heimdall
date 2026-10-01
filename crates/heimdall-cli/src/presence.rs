//! Asking the person at the computer before a lock is lifted (SPEC §6).
//!
//! An unlock widens what every agent with access to the vault can change, so
//! it has to come from a person rather than from a process acting for one. A
//! button an accessibility client can click, or a command an agent's shell can
//! run, proves nothing; Touch ID or the login password, asked for by macOS in
//! a dialog no other process can fill in, does.
//!
//! The question names what is being unlocked, in which vault, and which
//! program asked — so a prompt that appears while an agent is working says
//! plainly that the agent is the one asking.

use std::time::Duration;

use camino::Utf8PathBuf;
use heimdall_core::commands::Presence;
use heimdall_core::{Error, Result, Vault};

/// How long the question stays on screen. The desktop bridge waits a little
/// longer than this for the command as a whole.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const PROMPT_TIMEOUT: Duration = Duration::from_secs(90);

/// Whether a person must answer in this build. Release builds always ask; a
/// debug build can be told the answer for tests (see [`scripted_answer`]).
pub const REQUIRED: bool = !cfg!(debug_assertions);

/// The person at the computer, asked about one vault.
pub struct Person {
    vault_root: Utf8PathBuf,
}

impl Person {
    pub fn for_vault(vault: &Vault) -> Self {
        Self {
            vault_root: vault.root().to_owned(),
        }
    }
}

impl Presence for Person {
    fn confirm(&self, action: &str) -> Result<()> {
        if let Some(answer) = scripted_answer(&self.vault_root) {
            return answer;
        }
        let reason = match requester() {
            Some(name) => format!("{action} (requested by {name})"),
            None => action.to_string(),
        };
        ask(&reason)
    }
}

/// The answer a test scripted, if this is a debug build, the test asked for
/// one, and the vault is a throwaway one under the system temp directory.
///
/// Compiled out of release builds entirely. In a debug build it still cannot
/// unlock a real vault: those do not live in the temp directory, and moving one
/// there to get round this takes the same deliberate shell access as clearing
/// the file flag by hand.
#[cfg(debug_assertions)]
fn scripted_answer(vault_root: &Utf8PathBuf) -> Option<Result<()>> {
    let answer = std::env::var("HEIMDALL_TEST_PRESENCE").ok()?;
    let temp = std::fs::canonicalize(std::env::temp_dir()).ok()?;
    if !vault_root.as_std_path().starts_with(&temp) {
        return None;
    }
    Some(match answer.as_str() {
        "confirm" => Ok(()),
        _ => Err(Error::not_confirmed("the unlock was cancelled", "cancelled")),
    })
}

#[cfg(not(debug_assertions))]
fn scripted_answer(_vault_root: &Utf8PathBuf) -> Option<Result<()>> {
    None
}

/// The program that asked, for the prompt: the nearest ancestor process that
/// is not a shell, since an agent runs `heimdall unlock` through one.
#[cfg(target_os = "macos")]
fn requester() -> Option<String> {
    const SHELLS: [&str; 7] = ["sh", "bash", "zsh", "fish", "dash", "login", "env"];
    let mut pid = std::process::id();
    for _ in 0..6 {
        let info = bsd_info(pid)?;
        let parent = info.pbi_ppid;
        if parent <= 1 {
            return None;
        }
        let name = bsd_info(parent).map(|parent| process_name(&parent))?;
        if !name.is_empty() && !SHELLS.contains(&name.as_str()) {
            return Some(name);
        }
        pid = parent;
    }
    None
}

#[cfg(target_os = "macos")]
fn bsd_info(pid: u32) -> Option<libc::proc_bsdinfo> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    // SAFETY: `info` is a valid, writable buffer of exactly `size` bytes.
    let written = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            size,
        )
    };
    (written == size).then_some(info)
}

#[cfg(target_os = "macos")]
fn process_name(info: &libc::proc_bsdinfo) -> String {
    let bytes: Vec<u8> = info
        .pbi_name
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(not(target_os = "macos"))]
fn requester() -> Option<String> {
    None
}

/// Ask with Touch ID or the login password.
///
/// The reply arrives on a queue private to the framework, so a command-line
/// process can wait for it on a channel without running a loop of its own. A
/// process with no window-server session — over SSH, say — cannot be asked at
/// all, and that is a refusal, not a pass.
#[cfg(target_os = "macos")]
fn ask(reason: &str) -> Result<()> {
    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSError, NSString};
    use objc2_local_authentication::{LAContext, LAError, LAPolicy};

    let policy = LAPolicy::DeviceOwnerAuthentication;
    // SAFETY: plain Objective-C calls on an object this function owns.
    let context = unsafe { LAContext::new() };
    if let Err(error) = unsafe { context.canEvaluatePolicy_error(policy) } {
        return Err(unavailable(error.code()));
    }

    let (sender, receiver) = std::sync::mpsc::sync_channel::<std::result::Result<(), isize>>(1);
    let reply = RcBlock::new(move |success: Bool, error: *mut NSError| {
        let outcome = if success.as_bool() {
            Ok(())
        } else {
            // SAFETY: the framework passes either null or a valid NSError.
            Err(unsafe { error.as_ref() }.map_or(0, |error| error.code()))
        };
        let _ = sender.try_send(outcome);
    });
    let reason = NSString::from_str(reason);
    // SAFETY: the reply block only sends on a channel, which is thread-safe;
    // `context` outlives the evaluation because this function waits for it.
    unsafe { context.evaluatePolicy_localizedReason_reply(policy, &reason, &reply) };

    match receiver.recv_timeout(PROMPT_TIMEOUT) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(code))
            if [
                LAError::UserCancel.0,
                LAError::AppCancel.0,
                LAError::SystemCancel.0,
                LAError::UserFallback.0,
                LAError::AuthenticationFailed.0,
            ]
            .contains(&code) =>
        {
            Err(Error::not_confirmed("the unlock was cancelled", "cancelled"))
        }
        Ok(Err(code)) => Err(unavailable(code)),
        Err(_) => {
            // SAFETY: as above.
            unsafe { context.invalidate() };
            Err(Error::not_confirmed(
                "nobody answered the unlock prompt in time",
                "timeout",
            ))
        }
    }
}

#[cfg(target_os = "macos")]
fn unavailable(code: isize) -> Error {
    Error::not_confirmed(
        "this Mac could not ask for Touch ID or your password here (there may be no \
         logged-in screen session); unlock from the Heimdall app or a Terminal window",
        "unavailable",
    )
    .with_detail("os_code", code as i64)
}

/// Ask at the terminal. Weaker than the macOS dialog — a process that owns
/// the terminal can answer it — and documented as such (SPEC §6).
#[cfg(not(target_os = "macos"))]
fn ask(reason: &str) -> Result<()> {
    use std::io::{BufRead, Write};

    let tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .map_err(|_| {
            Error::not_confirmed("there is no terminal to ask for confirmation", "unavailable")
        })?;
    let mut writer = &tty;
    let _ = write!(writer, "heimdall: {reason}. Type \"unlock\" to confirm: ");
    let _ = writer.flush();
    let mut answer = String::new();
    std::io::BufReader::new(&tty)
        .read_line(&mut answer)
        .map_err(|_| Error::not_confirmed("no answer was read", "unavailable"))?;
    if answer.trim() == "unlock" {
        Ok(())
    } else {
        Err(Error::not_confirmed("the unlock was cancelled", "cancelled"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_builds_always_ask_a_person() {
        // The scripted answer exists for the test suite only.
        assert_eq!(REQUIRED, !cfg!(debug_assertions));
    }

    #[cfg(debug_assertions)]
    #[test]
    fn a_scripted_answer_never_reaches_a_vault_outside_the_temp_directory() {
        std::env::set_var("HEIMDALL_TEST_PRESENCE", "confirm");
        let home = Utf8PathBuf::from(std::env::var("HOME").unwrap()).join("Notes");
        assert!(scripted_answer(&home).is_none());
        let temp = Utf8PathBuf::from_path_buf(std::fs::canonicalize(std::env::temp_dir()).unwrap())
            .unwrap()
            .join("vault");
        assert!(matches!(scripted_answer(&temp), Some(Ok(()))));
        std::env::remove_var("HEIMDALL_TEST_PRESENCE");
    }
}
