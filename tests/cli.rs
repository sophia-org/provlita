//! The protected serve entry selects only the 9P file endpoint. These cases
//! stop before socket or GPU access; authentication cases read local config.
use std::process::Command;

fn serve(env: &[(&str, &str)]) -> (i32, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_provlita"))
        .arg("--serve")
        .env_clear()
        .envs(env.iter().copied())
        .output()
        .unwrap();
    (
        output.status.code().unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

#[test]
fn retired_socket_variable_is_refused_even_empty_or_beside_the_file_endpoint() {
    for retired in ["/nonexistent/shell.sock", ""] {
        let (code, stderr) = serve(&[
            ("SOPHIA_SHELL_SOCKET", retired),
            ("SOPHIA_SHELL_9P_SOCKET", "/nonexistent/shell-9p.sock"),
        ]);
        assert_eq!(code, 2);
        assert!(
            stderr.contains("SOPHIA_SHELL_SOCKET is unsupported; use SOPHIA_SHELL_9P_SOCKET"),
            "{stderr}"
        );
    }
}

#[test]
fn file_endpoint_is_required_and_must_be_nonempty() {
    for env in [&[][..], &[("SOPHIA_SHELL_9P_SOCKET", "")][..]] {
        let (code, stderr) = serve(env);
        assert_eq!(code, 2);
        assert!(
            stderr.contains("SOPHIA_SHELL_9P_SOCKET is required"),
            "{stderr}"
        );
    }
}

#[test]
fn malformed_or_closed_launch_descriptor_never_falls_back_to_plain_attach() {
    let directory = std::env::temp_dir().join(format!("dock-auth-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let socket = directory.join("role");
    let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    for value in ["", "2", "-1", "invalid", "2147483647"] {
        let (code, stderr) = serve(&[
            ("SOPHIA_SHELL_9P_SOCKET", socket.to_str().unwrap()),
            (
                "SOPHIA_SHELL_CONFIG",
                concat!(env!("CARGO_MANIFEST_DIR"), "/examples/minimal/config.kdl"),
            ),
            ("SOPHIA_SHELL_BAR_THICKNESS", "48"),
            ("SOPHIA_LAUNCH_AUTH_FD", value),
        ]);
        assert_eq!(code, 2);
        assert!(
            stderr.contains("SOPHIA_LAUNCH_AUTH_FD") || stderr.contains("launch channel:"),
            "{stderr}"
        );
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock,
            "invalid launch capability must not open the role transport"
        );
    }
    drop(listener);
    std::fs::remove_dir_all(directory).unwrap();
}
