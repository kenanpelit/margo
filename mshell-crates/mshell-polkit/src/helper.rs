use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tracing::{debug, error};

const HELPER_SOCKET: &str = "/run/polkit/agent-helper.socket";

#[derive(Debug, Clone)]
pub enum HelperEvent {
    Request { prompt: String, echo: bool },
    Info(String),
    Error(String),
    Completed { success: bool },
}

pub struct PolkitAgentHelper {
    writer: tokio::io::WriteHalf<UnixStream>,
}

impl PolkitAgentHelper {
    pub async fn connect(
        username: &str,
        cookie: &str,
    ) -> std::io::Result<(Self, tokio::sync::mpsc::Receiver<HelperEvent>)> {
        let stream = UnixStream::connect(HELPER_SOCKET).await?;
        let (reader, mut writer) = tokio::io::split(stream);

        // Write username and cookie — helper reads these first
        writer.write_all(format!("{username}\n").as_bytes()).await?;
        writer.write_all(format!("{cookie}\n").as_bytes()).await?;
        writer.flush().await?;

        let (tx, rx) = tokio::sync::mpsc::channel(16);

        tokio::spawn(async move {
            let buf_reader = BufReader::new(reader);
            let mut lines = buf_reader.lines();
            let mut completed = false;

            while let Ok(Some(line)) = lines.next_line().await {
                debug!("[polkit-helper] {line}");

                let event = parse_helper_line(&line);
                if matches!(event, HelperEvent::Completed { .. }) {
                    completed = true;
                }
                if tx.send(event).await.is_err() {
                    break;
                }
                if completed {
                    break;
                }
            }

            if !completed {
                let _ = tx.send(HelperEvent::Completed { success: false }).await;
            }
        });

        Ok((Self { writer }, rx))
    }

    pub async fn respond(&mut self, password: &str) -> std::io::Result<()> {
        self.writer.write_all(password.as_bytes()).await?;
        self.writer.write_all(b"\n").await?;
        self.writer.flush().await
    }
}

fn parse_helper_line(line: &str) -> HelperEvent {
    if line == "SUCCESS" {
        return HelperEvent::Completed { success: true };
    }
    if line == "FAILURE" {
        return HelperEvent::Completed { success: false };
    }
    if let Some(rest) = line.strip_prefix("PAM_PROMPT_ECHO_OFF ") {
        return HelperEvent::Request {
            prompt: rest.to_string(),
            echo: false,
        };
    }
    if let Some(rest) = line.strip_prefix("PAM_PROMPT_ECHO_ON ") {
        return HelperEvent::Request {
            prompt: rest.to_string(),
            echo: true,
        };
    }
    if let Some(rest) = line.strip_prefix("PAM_TEXT_INFO ") {
        return HelperEvent::Info(rest.to_string());
    }
    if let Some(rest) = line.strip_prefix("PAM_ERROR_MSG ") {
        return HelperEvent::Error(rest.to_string());
    }

    error!("[polkit-helper] unrecognized line: {line}");
    HelperEvent::Info(line.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_and_failure_terminate_with_the_right_flag() {
        // Mis-mapping either verdict would authenticate on failure or reject
        // on success — the whole point of the agent.
        assert!(matches!(
            parse_helper_line("SUCCESS"),
            HelperEvent::Completed { success: true }
        ));
        assert!(matches!(
            parse_helper_line("FAILURE"),
            HelperEvent::Completed { success: false }
        ));
    }

    #[test]
    fn echo_off_prompt_hides_input() {
        // ECHO_OFF is the password prompt — `echo` must be false so the entry
        // masks the characters.
        match parse_helper_line("PAM_PROMPT_ECHO_OFF Password: ") {
            HelperEvent::Request { prompt, echo } => {
                assert_eq!(prompt, "Password: ");
                assert!(!echo);
            }
            other => panic!("expected Request, got {other:?}"),
        }
    }

    #[test]
    fn echo_on_prompt_shows_input() {
        match parse_helper_line("PAM_PROMPT_ECHO_ON Username:") {
            HelperEvent::Request { prompt, echo } => {
                assert_eq!(prompt, "Username:");
                assert!(echo, "ECHO_ON must show the typed text");
            }
            other => panic!("expected Request, got {other:?}"),
        }
    }

    #[test]
    fn info_and_error_messages_strip_their_prefix() {
        match parse_helper_line("PAM_TEXT_INFO Insert your smartcard") {
            HelperEvent::Info(msg) => assert_eq!(msg, "Insert your smartcard"),
            other => panic!("expected Info, got {other:?}"),
        }
        match parse_helper_line("PAM_ERROR_MSG Account expired") {
            HelperEvent::Error(msg) => assert_eq!(msg, "Account expired"),
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn unrecognised_lines_degrade_to_info_verbatim() {
        // An unknown protocol line must not be treated as SUCCESS/FAILURE or
        // as a prompt — it surfaces as Info carrying the raw text.
        match parse_helper_line("SOMETHING ELSE") {
            HelperEvent::Info(msg) => assert_eq!(msg, "SOMETHING ELSE"),
            other => panic!("expected Info, got {other:?}"),
        }
    }

    #[test]
    fn prefixes_require_the_trailing_space() {
        // The parser keys off `"PREFIX "` (with the space). A bare token that
        // merely starts like a keyword is NOT a prompt — it falls through to
        // Info, so a spoofed `SUCCESSFUL` can't read as a completion.
        assert!(matches!(
            parse_helper_line("PAM_PROMPT_ECHO_OFF"),
            HelperEvent::Info(_)
        ));
        assert!(matches!(
            parse_helper_line("SUCCESSFUL"),
            HelperEvent::Info(_)
        ));
    }
}
