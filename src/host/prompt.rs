//! The terminal port: the `Prompter` trait the wizard asks through, and `Terminal`, which cliclack
//! draws on stderr.

use std::io;
use zeroize::Zeroizing;

pub struct Choice {
    pub label: String,
    pub hint: String,
}

impl Choice {
    pub fn new(label: impl Into<String>, hint: impl Into<String>) -> Choice {
        Choice {
            label: label.into(),
            hint: hint.into(),
        }
    }
}

pub trait Prompter {
    fn intro(&mut self, title: &str) -> io::Result<()>;
    fn info(&mut self, text: &str) -> io::Result<()>;
    fn warn(&mut self, text: &str) -> io::Result<()>;
    fn note(&mut self, title: &str, body: &str) -> io::Result<()>;
    fn select(&mut self, prompt: &str, choices: &[Choice], initial: usize) -> io::Result<usize>;
    fn multiselect(
        &mut self,
        prompt: &str,
        choices: &[Choice],
        initial: &[usize],
    ) -> io::Result<Vec<usize>>;
    /// Empty Enter takes `default`; `check` runs on the final value and its Err is shown under the prompt.
    fn input(
        &mut self,
        prompt: &str,
        default: &str,
        check: fn(&str) -> Result<(), String>,
    ) -> io::Result<String>;
    fn password(
        &mut self,
        prompt: &str,
        check: fn(&str) -> Result<(), String>,
    ) -> io::Result<Zeroizing<String>>;
    fn confirm(&mut self, prompt: &str, initial: bool) -> io::Result<bool>;
    /// Runs `work` under a spinner; stops with `done(&ok)` or shows the error text.
    fn spin<T>(
        &mut self,
        message: &str,
        work: impl FnOnce() -> Result<T, String>,
        done: impl FnOnce(&T) -> String,
    ) -> Result<T, String>;
    /// Runs `work` under a progress bar of `total` bytes; `work` reports the bytes done so far.
    /// Stops with `done(&ok)` or shows the error text, like `spin`.
    fn progress<T>(
        &mut self,
        message: &str,
        total: u64,
        work: impl FnOnce(&mut dyn FnMut(u64)) -> Result<T, String>,
        done: impl FnOnce(&T) -> String,
    ) -> Result<T, String>;
    fn outro(&mut self, text: &str) -> io::Result<()>;
    fn cancel(&mut self, text: &str) -> io::Result<()>;
}

/// Keeps the terminal from echoing while it lives. `console` only goes raw
/// inside each key read (unix_term.rs `read_single_key`) and restores cooked
/// mode, ECHO on, between reads, so a key pasted outside a read is echoed by
/// the line discipline.
struct NoEcho {
    tty: std::fs::File,
    saved: libc::termios,
}

impl NoEcho {
    fn on() -> Option<NoEcho> {
        use std::os::fd::AsRawFd;
        let tty = std::fs::File::open("/dev/tty").ok()?;
        let fd = tty.as_raw_fd();
        let mut saved = std::mem::MaybeUninit::<libc::termios>::uninit();
        // SAFETY: fd is open and saved is a valid out pointer.
        if unsafe { libc::tcgetattr(fd, saved.as_mut_ptr()) } != 0 {
            return None;
        }
        // SAFETY: tcgetattr filled it.
        let saved = unsafe { saved.assume_init() };
        let mut quiet = saved;
        quiet.c_lflag &= !libc::ECHO;
        // SAFETY: fd is open and quiet is a valid termios.
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &quiet) } != 0 {
            return None;
        }
        Some(NoEcho { tty, saved })
    }
}

impl Drop for NoEcho {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        // SAFETY: the fd is open and saved came from tcgetattr.
        unsafe { libc::tcsetattr(self.tty.as_raw_fd(), libc::TCSANOW, &self.saved) };
    }
}

/// The cliclack adapter; every prompt draws on stderr.
pub struct Terminal;

impl Prompter for Terminal {
    fn intro(&mut self, title: &str) -> io::Result<()> {
        cliclack::intro(title)
    }

    fn info(&mut self, text: &str) -> io::Result<()> {
        cliclack::log::info(text)
    }

    fn warn(&mut self, text: &str) -> io::Result<()> {
        cliclack::log::warning(text)
    }

    fn note(&mut self, title: &str, body: &str) -> io::Result<()> {
        cliclack::note(title, body)
    }

    fn select(&mut self, prompt: &str, choices: &[Choice], initial: usize) -> io::Result<usize> {
        let mut select = cliclack::select(prompt).initial_value(initial);
        for (i, choice) in choices.iter().enumerate() {
            select = select.item(i, &choice.label, &choice.hint);
        }
        select.interact()
    }

    fn multiselect(
        &mut self,
        prompt: &str,
        choices: &[Choice],
        initial: &[usize],
    ) -> io::Result<Vec<usize>> {
        let mut select = cliclack::multiselect(prompt)
            .required(false)
            .initial_values(initial.to_vec());
        for (i, choice) in choices.iter().enumerate() {
            select = select.item(i, &choice.label, &choice.hint);
        }
        select.interact()
    }

    fn input(
        &mut self,
        prompt: &str,
        default: &str,
        check: fn(&str) -> Result<(), String>,
    ) -> io::Result<String> {
        let mut input = cliclack::input(prompt);
        input = if default.is_empty() {
            input.required(false)
        } else {
            input.default_input(default)
        };
        input.validate(move |s: &String| check(s)).interact()
    }

    fn password(
        &mut self,
        prompt: &str,
        check: fn(&str) -> Result<(), String>,
    ) -> io::Result<Zeroizing<String>> {
        let _quiet = NoEcho::on();
        cliclack::password(prompt)
            .validate(move |s: &String| check(s))
            .interact()
            .map(Zeroizing::new)
    }

    fn confirm(&mut self, prompt: &str, initial: bool) -> io::Result<bool> {
        cliclack::confirm(prompt).initial_value(initial).interact()
    }

    fn spin<T>(
        &mut self,
        message: &str,
        work: impl FnOnce() -> Result<T, String>,
        done: impl FnOnce(&T) -> String,
    ) -> Result<T, String> {
        let spinner = cliclack::spinner();
        spinner.start(message);
        match work() {
            Ok(value) => {
                spinner.stop(done(&value));
                Ok(value)
            }
            Err(message) => {
                spinner.error(&message);
                Err(message)
            }
        }
    }

    fn progress<T>(
        &mut self,
        message: &str,
        total: u64,
        work: impl FnOnce(&mut dyn FnMut(u64)) -> Result<T, String>,
        done: impl FnOnce(&T) -> String,
    ) -> Result<T, String> {
        let bar = cliclack::progress_bar(total).with_download_template();
        bar.start(message);
        match work(&mut |n| bar.set_position(n)) {
            Ok(value) => {
                bar.stop(done(&value));
                Ok(value)
            }
            Err(message) => {
                bar.error(&message);
                Err(message)
            }
        }
    }

    fn outro(&mut self, text: &str) -> io::Result<()> {
        cliclack::outro(text)
    }

    fn cancel(&mut self, text: &str) -> io::Result<()> {
        cliclack::outro_cancel(text)
    }
}
