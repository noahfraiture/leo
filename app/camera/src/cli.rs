use std::{ffi::OsStr, io, net::SocketAddr};

pub(crate) fn parse(args: impl IntoIterator<Item = impl AsRef<OsStr>>) -> io::Result<SocketAddr> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidInput, "expected --bind <SocketAddr>");
    let mut args = args.into_iter();
    let flag = args.next().ok_or_else(invalid)?;
    let address = args.next().ok_or_else(invalid)?;

    if flag.as_ref() != OsStr::new("--bind") || args.next().is_some() {
        return Err(invalid());
    }

    address
        .as_ref()
        .to_str()
        .ok_or_else(invalid)?
        .parse()
        .map_err(|_| invalid())
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;

    use super::parse;

    fn assert_invalid(args: &[&str]) {
        assert_eq!(parse(args).unwrap_err().kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn parses_bind_address() {
        assert_eq!(
            parse(["--bind", "127.0.0.1:3000"]).unwrap(),
            "127.0.0.1:3000".parse().unwrap()
        );
    }

    #[test]
    fn rejects_missing_arguments() {
        assert_invalid(&[]);
        assert_invalid(&["--bind"]);
    }

    #[test]
    fn rejects_malformed_address() {
        assert_invalid(&["--bind", "localhost"]);
    }

    #[test]
    fn rejects_wrong_flag() {
        assert_invalid(&["--address", "127.0.0.1:3000"]);
    }

    #[test]
    fn rejects_extra_arguments() {
        assert_invalid(&["--bind", "127.0.0.1:3000", "extra"]);
    }
}
