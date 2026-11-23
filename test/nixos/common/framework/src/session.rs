use std::{
    borrow::Cow,
    time::{Duration, Instant},
};

use rexpect::{reader::Regex, session::PtySession};
use uuid::Uuid;

use super::{Error, clean_output, truncate_output_for_error};

pub struct SessionDesc {
    prompt: Cow<'static, str>,
    enter_cmd: Cow<'static, str>,
    exit_cmd: Cow<'static, str>,
}

impl SessionDesc {
    pub fn new(
        prompt: impl Into<Cow<'static, str>>,
        enter_cmd: impl Into<Cow<'static, str>>,
        exit_cmd: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self {
            prompt: prompt.into(),
            enter_cmd: enter_cmd.into(),
            exit_cmd: exit_cmd.into(),
        }
    }
}

pub struct Session {
    uuid: String,
    desc: SessionDesc,
    pty_session: PtySession,
}

impl Session {
    pub(super) fn new(desc: SessionDesc, pty_session: PtySession) -> Self {
        Self {
            uuid: Uuid::new_v4().to_string(),
            desc,
            pty_session,
        }
    }

    pub fn run_cmd(&mut self, command: &str) -> Result<(), Error> {
        println!("--> Running: {}", command);
        self.pty_session.send_line(command).map_err(Error::from)?;
        self.pty_session.exp_string(command).map_err(Error::from)?;

        if let Err(error) = self
            .pty_session
            .exp_string(&self.desc.prompt)
            .map_err(Error::from)
        {
            Self::output_error(&error);
            return Err(error);
        }

        Ok(())
    }

    pub fn run_cmd_and_expect(&mut self, command: &str, expected: &str) -> Result<(), Error> {
        println!("--> Running: {} (expecting: {})", command, expected);
        self.pty_session.send_line(command).map_err(Error::from)?;
        self.pty_session.exp_string(command).map_err(Error::from)?;

        match self
            .pty_session
            .exp_string(&self.desc.prompt)
            .map_err(Error::from)
        {
            Ok(unread) => {
                let cleaned_unread = clean_output(&unread);
                if !cleaned_unread.contains(expected) {
                    let error = Error::UnexpectedOutput {
                        expected: expected.to_string(),
                        got: truncate_output_for_error(&cleaned_unread),
                    };
                    Self::output_error(&error);
                    return Err(error);
                }
            }
            Err(e) => {
                Self::output_error(&e);
                return Err(e);
            }
        }

        Ok(())
    }

    pub fn run_cmd_and_expect_regex(
        &mut self,
        command: &str,
        expected: &Regex,
    ) -> Result<(), Error> {
        let command_output = self.run_cmd_and_collect_output(command)?;

        if command_output.exit_status != 0 {
            return Err(Error::NonZeroExit {
                exit_status: command_output.exit_status,
                output: truncate_output_for_error(&command_output.output),
            });
        }

        if !expected.is_match(&command_output.output) {
            let error = Error::UnexpectedOutput {
                expected: expected.as_str().to_string(),
                got: truncate_output_for_error(&command_output.output),
            };
            Self::output_error(&error);
            return Err(error);
        }

        Ok(())
    }

    pub fn enter_session_and_run<F>(&mut self, desc: SessionDesc, test_ops: F) -> Result<(), Error>
    where
        F: FnOnce(&mut Session) -> Result<(), Error>,
    {
        let old_desc = std::mem::replace(&mut self.desc, desc);

        if let Err(e) = self.run_cmd(&self.desc.enter_cmd.clone()) {
            self.desc = old_desc;
            return Err(e);
        }

        let res = test_ops(self);

        let exit_cmd = self.desc.exit_cmd.clone();
        self.desc = old_desc;
        let exit_res = self.run_cmd(&exit_cmd);

        res?;
        exit_res?;

        Ok(())
    }

    pub fn with_background_process<F>(
        &mut self,
        background_process: BackgroundProcess,
        test_ops: F,
    ) -> Result<(), Error>
    where
        F: FnOnce(&mut Session) -> Result<(), Error>,
    {
        let guard = BackgroundProcessGuard {
            session: self,
            background_process,
        };

        let command_output = guard
            .session
            .run_cmd_and_collect_output(&guard.background_process.start)?;
        if command_output.exit_status != 0 {
            return Err(Error::NonZeroExit {
                exit_status: command_output.exit_status,
                output: truncate_output_for_error(&command_output.output),
            });
        }

        guard.session.wait_until_check_matches(
            &guard.background_process.ready,
            READY_TIMEOUT,
            "background process to become ready",
        )?;

        test_ops(guard.session)
    }

    pub(super) fn shutdown(&mut self) -> Result<(), Error> {
        self.pty_session
            .send_line(&self.desc.exit_cmd)
            .map_err(Error::from)?;

        self.pty_session.process().wait().map_err(Error::from)?;

        Ok(())
    }

    fn output_error(error: &Error) {
        match error {
            Error::Pty(rexpect::error::Error::EOF {
                expected,
                got,
                exit_code,
            }) => {
                println!("=== EOF Error Details ===");
                println!("Expected: {}", expected);
                println!("Got: {}", got);
                println!("Exit code: {:?}", exit_code);
                println!("========================");
            }
            Error::Timeout {
                expected,
                got,
                timeout,
            } => {
                println!("=== Timeout Error Details ===");
                println!("Expected: {}", expected);
                println!("Got: {}", got);
                println!("Timeout: {:?}", timeout);
                println!("============================");
            }
            Error::UnexpectedOutput { expected, got } => {
                println!("=== Unexpected Output ===");
                println!("Expected: {}", expected);
                println!("Output before prompt:\n{}", got);
                println!("=========================");
            }
            Error::Protocol { reason, got } => {
                println!("=== Protocol Error Details ===");
                println!("Reason: {}", reason);
                println!("Got: {}", got);
                println!("==============================");
            }
            Error::NonZeroExit {
                exit_status,
                output,
            } => {
                println!("=== Command Exit Error Details ===");
                println!("Exit status: {}", exit_status);
                println!("Output:\n{}", output);
                println!("==================================");
            }
            Error::Pty(reason) => {
                println!("=== PTY Error Details ===");
                println!("Reason: {}", reason);
                println!("=========================");
            }
            Error::Aggregated {
                summary,
                collections,
            } => {
                println!("=== Aggregated Error Details ===");
                println!("Summary: {}", summary);
                for (name, error) in collections {
                    println!("--- {}", name);
                    Self::output_error(error);
                }
                println!("================================");
            }
        }
    }

    fn run_cmd_and_collect_output(&mut self, command: &str) -> Result<CommandOutput, Error> {
        let exit_marker = self.uuid.as_str();
        let quoted_command = format!("'{}'", command.replace('\'', r#"'"'"'"#));
        let wrapped_command = format!(
            r#"eval -- {}; __exit_code=$?; printf '\n{}%s\n' "$__exit_code""#,
            quoted_command, exit_marker
        );

        println!("--> Running: {}", command);
        self.pty_session
            .send_line(&wrapped_command)
            .map_err(Error::from)?;
        self.pty_session
            .exp_string(&wrapped_command)
            .map_err(Error::from)?;

        let unread = match self
            .pty_session
            .exp_string(&self.desc.prompt)
            .map_err(Error::from)
        {
            Ok(unread) => unread,
            Err(error) => {
                Self::output_error(&error);
                return Err(error);
            }
        };

        let (command_output, exit_status) = Self::parse_command_output(&unread, exit_marker)?;

        Ok(CommandOutput {
            exit_status,
            output: command_output,
        })
    }

    fn parse_command_output(output: &str, exit_marker: &str) -> Result<(String, i32), Error> {
        let sanitize_output = |str| truncate_output_for_error(&clean_output(str));

        let Some((command_output, exit_status_text)) = output.rsplit_once(exit_marker) else {
            return Err(Error::Protocol {
                reason: "missing exit status marker".to_string(),
                got: sanitize_output(output),
            });
        };

        let Some(exit_status_line) = exit_status_text.lines().next() else {
            return Err(Error::Protocol {
                reason: "missing numeric exit status".to_string(),
                got: sanitize_output(command_output),
            });
        };
        let Ok(exit_status) = exit_status_line.parse::<i32>() else {
            return Err(Error::Protocol {
                reason: format!("invalid numeric exit status: {}", exit_status_line),
                got: sanitize_output(command_output),
            });
        };

        Ok((clean_output(command_output).trim().to_string(), exit_status))
    }

    fn stop_background_process(
        &mut self,
        background_process: &BackgroundProcess,
    ) -> Result<(), Error> {
        let command_output = self.run_cmd_and_collect_output(&background_process.stop)?;
        if command_output.exit_status != 0 {
            return Err(Error::NonZeroExit {
                exit_status: command_output.exit_status,
                output: truncate_output_for_error(&command_output.output),
            });
        }

        self.wait_until_check_matches(
            &background_process.stopped,
            STOP_TIMEOUT,
            "background process to stop",
        )
    }

    pub fn wait_until_check_matches(
        &mut self,
        command_check: &CommandCheck,
        timeout: Duration,
        expected_state: &str,
    ) -> Result<(), Error> {
        let start_time = Instant::now();

        loop {
            let command_output = self.run_cmd_and_collect_output(&command_check.cmd)?;

            if command_output.exit_status == 0
                && command_output
                    .output
                    .contains(command_check.expect.as_ref())
            {
                return Ok(());
            }

            if start_time.elapsed() >= timeout {
                return Err(Error::Timeout {
                    expected: expected_state.to_string(),
                    got: format!(
                        "exit status: {}\nexpected output: {}\nactual output:\n{}",
                        command_output.exit_status,
                        command_check.expect,
                        truncate_output_for_error(&command_output.output)
                    ),
                    timeout,
                });
            }

            std::thread::sleep(POLL_INTERVAL);
        }
    }
}

pub struct BackgroundProcess {
    start: Cow<'static, str>,
    ready: CommandCheck,
    stop: Cow<'static, str>,
    stopped: CommandCheck,
}

impl BackgroundProcess {
    pub fn new(
        start: impl Into<Cow<'static, str>>,
        ready: CommandCheck,
        stop: impl Into<Cow<'static, str>>,
        stopped: CommandCheck,
    ) -> Self {
        Self {
            start: start.into(),
            ready,
            stop: stop.into(),
            stopped,
        }
    }
}

pub struct CommandCheck {
    cmd: Cow<'static, str>,
    expect: Cow<'static, str>,
}

impl CommandCheck {
    pub fn new(cmd: impl Into<Cow<'static, str>>, expect: impl Into<Cow<'static, str>>) -> Self {
        Self {
            cmd: cmd.into(),
            expect: expect.into(),
        }
    }
}

struct CommandOutput {
    exit_status: i32,
    output: String,
}

struct BackgroundProcessGuard<'s> {
    session: &'s mut Session,
    background_process: BackgroundProcess,
}

impl Drop for BackgroundProcessGuard<'_> {
    fn drop(&mut self) {
        let _ = self
            .session
            .stop_background_process(&self.background_process);
    }
}

const READY_TIMEOUT: Duration = Duration::from_secs(30);
const STOP_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_secs(1);

#[cfg(test)]
mod tests {
    use super::{Error, Session};

    #[test]
    fn parse_command_output_extracts_status_from_marker() {
        let exit_marker = "00000000-0000-0000-0000-000000000000";
        let command_output = format!("hello\n{}17\n", exit_marker);

        let (output, exit_status) =
            Session::parse_command_output(&command_output, exit_marker).unwrap();

        assert_eq!(output, "hello");
        assert_eq!(exit_status, 17);
    }

    #[test]
    fn parse_command_output_rejects_missing_marker() {
        let exit_marker = "00000000-0000-0000-0000-000000000000";
        let error = Session::parse_command_output("hello", exit_marker).unwrap_err();

        match error {
            Error::Protocol { reason, got } => {
                assert_eq!(reason, "missing exit status marker");
                assert_eq!(got, "hello");
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }
}
