use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug, Clone)]
#[command(
    name = "flowcase_audio_out",
    version,
    about = "MPEG-TS WebSocket relay for Flowcase desktop audio.",
    long_about = "Listens on <stream-port> for HTTP POSTs of an MPEG-TS \
                  stream (typically from ffmpeg) and fans the bytes out \
                  to all WebSocket clients connected on <ws-port>."
)]
pub struct Cli {
    /// URL-path secret. Only POSTs to /<secret> on the ingest port are accepted.
    pub secret: String,

    /// Plain HTTP port that ffmpeg POSTs the MPEG-TS stream to.
    pub stream_port: u16,

    /// TLS-only WebSocket port that browsers connect to.
    pub ws_port: u16,

    /// PEM-encoded TLS certificate.
    pub ssl_cert: PathBuf,

    /// PEM-encoded TLS private key.
    pub ssl_key: PathBuf,

    /// Optional HTTP Basic auth token in `user:pass` form.
    pub auth_token: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_invocation_from_droplet_startup() {
        let cli = Cli::try_parse_from([
            "flowcase_audio_out",
            "foo",
            "8081",
            "4901",
            "/tmp/cert.pem",
            "/tmp/key.pem",
            "flowcase_user:abc",
        ])
        .expect("expected the canonical 6-positional invocation to parse");

        assert_eq!(cli.secret, "foo");
        assert_eq!(cli.stream_port, 8081);
        assert_eq!(cli.ws_port, 4901);
        assert_eq!(cli.ssl_cert, PathBuf::from("/tmp/cert.pem"));
        assert_eq!(cli.ssl_key, PathBuf::from("/tmp/key.pem"));
        assert_eq!(cli.auth_token.as_deref(), Some("flowcase_user:abc"));
    }

    #[test]
    fn auth_token_is_optional() {
        let cli = Cli::try_parse_from([
            "flowcase_audio_out",
            "foo",
            "8081",
            "4901",
            "/tmp/cert.pem",
            "/tmp/key.pem",
        ])
        .expect("expected invocation without auth_token to parse");

        assert!(cli.auth_token.is_none());
    }
}
