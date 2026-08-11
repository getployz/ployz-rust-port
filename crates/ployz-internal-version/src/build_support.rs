use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;

const SENTINEL_NAME: &str = "ployz-version-always-missing";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DirectiveValueError {
    MissingOutDir,
    NonUnicodeOutDir,
    LineBreak,
}

impl fmt::Display for DirectiveValueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingOutDir => formatter.write_str("Cargo did not provide OUT_DIR"),
            Self::NonUnicodeOutDir => formatter.write_str("OUT_DIR is not valid Unicode"),
            Self::LineBreak => formatter.write_str("Cargo directive value contains CR or LF"),
        }
    }
}

impl Error for DirectiveValueError {}

pub(crate) fn missing_sentinel(out_dir: Option<OsString>) -> Result<String, DirectiveValueError> {
    let out_dir = out_dir.ok_or(DirectiveValueError::MissingOutDir)?;
    let path = PathBuf::from(out_dir).join(SENTINEL_NAME);
    let path = path
        .into_os_string()
        .into_string()
        .map_err(|_| DirectiveValueError::NonUnicodeOutDir)?;
    single_line(&path).map(str::to_owned)
}

pub(crate) fn single_line(value: &str) -> Result<&str, DirectiveValueError> {
    if value.contains(['\r', '\n']) {
        Err(DirectiveValueError::LineBreak)
    } else {
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentinel_is_missing_and_beneath_out_dir() {
        let out_dir = OsString::from("/tmp/ployz-out");
        assert_eq!(
            missing_sentinel(Some(out_dir)).unwrap(),
            "/tmp/ployz-out/ployz-version-always-missing"
        );
    }

    #[test]
    fn dynamic_directive_values_reject_every_line_ending() {
        for value in ["one\ntwo", "one\rtwo", "one\r\ntwo"] {
            assert_eq!(single_line(value), Err(DirectiveValueError::LineBreak));
        }
    }

    #[test]
    fn absent_out_dir_is_rejected() {
        assert_eq!(
            missing_sentinel(None),
            Err(DirectiveValueError::MissingOutDir)
        );
    }

    #[cfg(unix)]
    #[test]
    fn non_unicode_out_dir_is_rejected() {
        use std::os::unix::ffi::OsStringExt;

        let out_dir = OsString::from_vec(vec![b'/', b't', b'm', b'p', b'/', 0xff]);
        assert_eq!(
            missing_sentinel(Some(out_dir)),
            Err(DirectiveValueError::NonUnicodeOutDir)
        );
    }
}
