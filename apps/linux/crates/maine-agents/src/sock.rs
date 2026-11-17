//! Small socket helpers shared by the server and the bridge's client.

use std::io;
use std::os::unix::net::UnixStream;

/// Writes every byte, retrying short writes and interrupts. Uses `MSG_NOSIGNAL`
/// so a closed peer is an error, never a `SIGPIPE`, even in a host process
/// (the Qt app) that has not ignored the signal.
pub(crate) fn send_all(stream: &UnixStream, mut data: &[u8]) -> io::Result<()> {
    while !data.is_empty() {
        match rustix::net::send(stream, data, rustix::net::SendFlags::NOSIGNAL) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(written) => data = &data[written..],
            Err(rustix::io::Errno::INTR) => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}
