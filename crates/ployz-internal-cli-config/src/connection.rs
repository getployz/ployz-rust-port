use std::{fmt, net::SocketAddr, num::ParseIntError};

use ployz_internal_secret::Secret;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Ways the CLI can reach one Ployz machine, plus connection metadata.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
pub struct MachineConnection {
    /// Destination for the system SSH client, which is the default SSH method.
    #[serde(default, skip_serializing_if = "SshDestination::is_empty")]
    pub ssh: SshDestination,
    /// Backward-compatible alias for [`Self::ssh`].
    #[serde(default, skip_serializing_if = "SshDestination::is_empty")]
    pub ssh_cli: SshDestination,
    /// Destination for the built-in SSH implementation.
    #[serde(default, skip_serializing_if = "SshDestination::is_empty")]
    pub ssh_go: SshDestination,
    /// Optional SSH private-key path.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ssh_key_file: String,
    /// Machine API TCP endpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tcp: Option<SocketAddr>,
    /// Machine API Unix-socket path.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub unix: String,
    /// Machine host metadata.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub host: String,
    /// Machine public key.
    #[serde(default, skip_serializing_if = "Secret::is_empty")]
    pub public_key: Secret,
    /// Stable machine identifier.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub machine_id: String,
}

impl MachineConnection {
    /// Ensures exactly one usable connection method is configured.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectionValidationError::NoMethod`] when none is set and
    /// [`ConnectionValidationError::MultipleMethods`] when more than one is set.
    pub fn validate(&self) -> Result<(), ConnectionValidationError> {
        let set_count = [
            !self.ssh.is_empty(),
            !self.ssh_cli.is_empty(),
            !self.ssh_go.is_empty(),
            self.tcp.is_some(),
            !self.unix.is_empty(),
        ]
        .into_iter()
        .filter(|is_set| *is_set)
        .count();

        match set_count {
            0 => Err(ConnectionValidationError::NoMethod),
            1 => Ok(()),
            _ => Err(ConnectionValidationError::MultipleMethods),
        }
    }
}

impl fmt::Display for MachineConnection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.ssh.is_empty() {
            write!(formatter, "ssh://{}", self.ssh)
        } else if !self.ssh_cli.is_empty() {
            write!(formatter, "ssh://{}", self.ssh_cli)
        } else if !self.ssh_go.is_empty() {
            write!(formatter, "ssh+go://{}", self.ssh_go)
        } else if let Some(address) = self.tcp {
            write!(formatter, "tcp://{address}")
        } else if !self.unix.is_empty() {
            write!(formatter, "unix://{}", self.unix)
        } else {
            formatter.write_str("unknown connection")
        }
    }
}

/// A machine connection has either no usable method or too many methods.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ConnectionValidationError {
    /// No connection method was set.
    #[error("no connection method specified (ssh, ssh_go, tcp, or unix required)")]
    NoMethod,
    /// Multiple connection methods were set.
    #[error("only one connection method allowed per connection (ssh, ssh_go, tcp, or unix)")]
    MultipleMethods,
}

/// An SSH destination in the canonical `user@host:port` form.
///
/// The user and port components are optional.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Deserialize, Serialize)]
#[serde(transparent)]
pub struct SshDestination(String);

impl SshDestination {
    /// Returns whether the destination is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Splits the destination into `(user, host, port)`.
    ///
    /// Missing users become an empty string and missing ports become zero. As
    /// in the Go source, malformed host/port syntax is treated as a host with
    /// no port; only a syntactically separated non-integer port is an error.
    ///
    /// # Errors
    ///
    /// Returns [`ParseSshDestinationError`] when a separated port is not a
    /// machine-sized integer.
    pub fn parse(&self) -> Result<(String, String, isize), ParseSshDestinationError> {
        let (user, host) = self
            .0
            .split_once('@')
            .map_or(("", self.0.as_str()), |(user, host)| (user, host));

        let Some((host, port)) = split_host_port(host) else {
            return Ok((user.to_owned(), host.to_owned(), 0));
        };

        let port = port.parse().map_err(|source| ParseSshDestinationError {
            port: port.to_owned(),
            source,
        })?;
        Ok((user.to_owned(), host.to_owned(), port))
    }
}

impl From<&str> for SshDestination {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for SshDestination {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl AsRef<str> for SshDestination {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SshDestination {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Constructs an SSH destination from its optional user and port components.
#[must_use]
pub fn new_ssh_destination(user: &str, host: &str, port: isize) -> SshDestination {
    let destination = if port == 0 {
        host.to_owned()
    } else if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };

    if user.is_empty() {
        destination.into()
    } else {
        format!("{user}@{destination}").into()
    }
}

/// A separated SSH port was not an integer.
#[derive(Debug, Error)]
#[error("parse SSH port '{port}': {source}")]
pub struct ParseSshDestinationError {
    port: String,
    #[source]
    source: ParseIntError,
}

fn split_host_port(destination: &str) -> Option<(&str, &str)> {
    if let Some(bracketed) = destination.strip_prefix('[') {
        let (host, suffix) = bracketed.split_once(']')?;
        let port = suffix.strip_prefix(':')?;
        if port.contains(':') {
            return None;
        }
        return Some((host, port));
    }

    let mut components = destination.split(':');
    let host = components.next()?;
    let port = components.next()?;
    if components.next().is_some() {
        return None;
    }
    Some((host, port))
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};

    use super::*;

    #[test]
    fn machine_connection_string_matches_method_precedence() {
        let cases = [
            (
                MachineConnection {
                    ssh: "user@host.com".into(),
                    ..Default::default()
                },
                "ssh://user@host.com",
            ),
            (
                MachineConnection {
                    ssh: "user@host.com:2222".into(),
                    ..Default::default()
                },
                "ssh://user@host.com:2222",
            ),
            (
                MachineConnection {
                    ssh_cli: "user@host.com".into(),
                    ..Default::default()
                },
                "ssh://user@host.com",
            ),
            (
                MachineConnection {
                    ssh_cli: "user@host.com:2222".into(),
                    ..Default::default()
                },
                "ssh://user@host.com:2222",
            ),
            (
                MachineConnection {
                    ssh_go: "user@host.com".into(),
                    ..Default::default()
                },
                "ssh+go://user@host.com",
            ),
            (
                MachineConnection {
                    ssh_go: "user@host.com:2222".into(),
                    ..Default::default()
                },
                "ssh+go://user@host.com:2222",
            ),
            (
                MachineConnection {
                    tcp: Some(SocketAddr::new(
                        IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
                        8080,
                    )),
                    ..Default::default()
                },
                "tcp://10.0.0.1:8080",
            ),
            (
                MachineConnection {
                    unix: "/run/ployz/ployz.sock".into(),
                    ..Default::default()
                },
                "unix:///run/ployz/ployz.sock",
            ),
            (MachineConnection::default(), "unknown connection"),
            (
                MachineConnection {
                    ssh: "user@host.com".into(),
                    machine_id: "ed98b9f7575308c340263cd279e3b568".into(),
                    ..Default::default()
                },
                "ssh://user@host.com",
            ),
            (
                MachineConnection {
                    ssh: "first".into(),
                    ssh_cli: "second".into(),
                    ssh_go: "third".into(),
                    tcp: Some("10.0.0.1:80".parse().expect("valid fixture")),
                    unix: "/last".into(),
                    ..Default::default()
                },
                "ssh://first",
            ),
        ];

        for (connection, expected) in cases {
            assert_eq!(connection.to_string(), expected);
        }
    }

    #[test]
    fn machine_connection_validate_requires_exactly_one_method() {
        let tcp = || Some("10.0.0.1:8080".parse().expect("valid fixture"));
        let valid = [
            MachineConnection {
                ssh: "user@host".into(),
                ..Default::default()
            },
            MachineConnection {
                ssh_cli: "user@host".into(),
                ..Default::default()
            },
            MachineConnection {
                ssh_go: "user@host".into(),
                ..Default::default()
            },
            MachineConnection {
                tcp: tcp(),
                ..Default::default()
            },
            MachineConnection {
                unix: "/path/to/socket".into(),
                ..Default::default()
            },
        ];
        for connection in valid {
            assert_eq!(connection.validate(), Ok(()));
        }

        assert_eq!(
            MachineConnection::default().validate(),
            Err(ConnectionValidationError::NoMethod)
        );
        assert_eq!(
            MachineConnection {
                ssh: "user@host".into(),
                ssh_cli: "user@host".into(),
                ..Default::default()
            }
            .validate(),
            Err(ConnectionValidationError::MultipleMethods)
        );
        assert_eq!(
            MachineConnection {
                ssh: "user@host".into(),
                ssh_go: "user@host".into(),
                ..Default::default()
            }
            .validate(),
            Err(ConnectionValidationError::MultipleMethods)
        );
        assert_eq!(
            MachineConnection {
                ssh: "user@host".into(),
                unix: "/path/to/socket".into(),
                ..Default::default()
            }
            .validate(),
            Err(ConnectionValidationError::MultipleMethods)
        );
        assert_eq!(
            MachineConnection {
                ssh: "user@host".into(),
                tcp: tcp(),
                ..Default::default()
            }
            .validate(),
            Err(ConnectionValidationError::MultipleMethods)
        );
        assert_eq!(
            MachineConnection {
                ssh_cli: "user@host".into(),
                tcp: tcp(),
                ..Default::default()
            }
            .validate(),
            Err(ConnectionValidationError::MultipleMethods)
        );
        assert_eq!(
            MachineConnection {
                ssh: "user@host".into(),
                ssh_cli: "user@host2".into(),
                tcp: tcp(),
                ..Default::default()
            }
            .validate(),
            Err(ConnectionValidationError::MultipleMethods)
        );
    }

    #[test]
    fn ssh_destination_constructs_canonical_forms() {
        assert_eq!(new_ssh_destination("", "host", 0).as_ref(), "host");
        assert_eq!(new_ssh_destination("user", "host", 0).as_ref(), "user@host");
        assert_eq!(new_ssh_destination("", "host", 22).as_ref(), "host:22");
        assert_eq!(
            new_ssh_destination("user", "::1", 2222).as_ref(),
            "user@[::1]:2222"
        );
        assert_eq!(
            new_ssh_destination("user", "host", -1).as_ref(),
            "user@host:-1"
        );
    }

    #[test]
    fn ssh_destination_parse_preserves_go_split_host_port_quirks() {
        let cases = [
            ("host", ("", "host", 0)),
            ("user@host", ("user", "host", 0)),
            ("host:22", ("", "host", 22)),
            ("user@host:2222", ("user", "host", 2222)),
            ("user@[::1]:22", ("user", "::1", 22)),
            ("::1", ("", "::1", 0)),
            ("user@host:22:33", ("user", "host:22:33", 0)),
            ("[::1]:22:33", ("", "[::1]:22:33", 0)),
            ("first@second@host", ("first", "second@host", 0)),
            (":22", ("", "", 22)),
        ];

        for (destination, expected) in cases {
            let actual = SshDestination::from(destination)
                .parse()
                .expect("valid fixture");
            assert_eq!(actual, (expected.0.into(), expected.1.into(), expected.2));
        }

        assert!(SshDestination::from("host:not-a-port").parse().is_err());
        assert!(SshDestination::from("host:").parse().is_err());
    }
}
