use super::*;
use std::{
    io::{Read, Write},
    os::fd::{AsRawFd, IntoRawFd},
};

#[test]
fn inherited_descriptor_is_strictly_bounded_and_never_stdio() {
    assert_eq!(descriptor(OsStr::new("3")).unwrap(), 3);
    assert_eq!(descriptor(OsStr::new("2147483647")).unwrap(), i32::MAX);
    for invalid in [
        "",
        "0",
        "1",
        "2",
        "-1",
        "+3",
        " 3",
        "3 ",
        "3x",
        "2147483648",
    ] {
        assert!(descriptor(OsStr::new(invalid)).is_err(), "{invalid:?}");
    }
    assert!(adopt(-1).is_err());
}

#[test]
fn adopted_generation_channel_is_cloexec_and_remains_usable() {
    let (source, mut peer) = UnixStream::pair().unwrap();
    let raw = source.into_raw_fd();
    assert_eq!(unsafe { libc::fcntl(raw, libc::F_SETFD, 0) }, 0);
    let mut stream = adopt(raw).unwrap();
    let flags = unsafe { libc::fcntl(stream.as_raw_fd(), libc::F_GETFD) };
    assert_ne!(flags & libc::FD_CLOEXEC, 0);
    peer.write_all(b"launch").unwrap();
    let mut received = [0; 6];
    stream.read_exact(&mut received).unwrap();
    assert_eq!(&received, b"launch");
}
