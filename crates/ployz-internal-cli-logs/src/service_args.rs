use std::collections::HashMap;
use std::error::Error;
use std::fmt;

/// A service and optional set of selected containers.
///
/// An empty container list selects every container belonging to the service.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceArg {
    pub service: String,
    pub containers: Vec<String>,
}

/// A malformed positional service argument.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseServiceArgError {
    message: String,
}

impl fmt::Display for ParseServiceArgError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for ParseServiceArgError {}

/// Groups `SERVICE` and `SERVICE/CONTAINER` arguments by service while
/// retaining first-seen service and container order.
pub fn parse_service_args<I, S>(args: I) -> Result<Vec<ServiceArg>, ParseServiceArgError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args = args.into_iter();
    let (lower_bound, _) = args.size_hint();
    let mut index_by_service: HashMap<String, usize> = HashMap::with_capacity(lower_bound);
    let mut all_containers: Vec<bool> = Vec::with_capacity(lower_bound);
    let mut result: Vec<ServiceArg> = Vec::with_capacity(lower_bound);

    for raw_arg in args {
        let arg = raw_arg.as_ref().trim();
        if arg.is_empty() {
            return Err(ParseServiceArgError {
                message: "empty service argument".to_owned(),
            });
        }

        let (service, container) = match arg.split_once('/') {
            Some(("", _)) => {
                return Err(ParseServiceArgError {
                    message: format!("invalid service argument '{arg}': service name is empty"),
                });
            }
            Some((_, "")) => {
                return Err(ParseServiceArgError {
                    message: format!(
                        "invalid service argument '{arg}': container name or ID is empty"
                    ),
                });
            }
            Some((service, container)) => (service, Some(container)),
            None => (arg, None),
        };

        if let Some(&index) = index_by_service.get(service) {
            match container {
                None => {
                    result[index].containers.clear();
                    all_containers[index] = true;
                }
                Some(container) if !all_containers[index] => {
                    result[index].containers.push(container.to_owned());
                }
                Some(_) => {}
            }
            continue;
        }

        let containers = container.into_iter().map(str::to_owned).collect();
        let index = result.len();
        result.push(ServiceArg {
            service: service.to_owned(),
            containers,
        });
        all_containers.push(container.is_none());
        index_by_service.insert(service.to_owned(), index);
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service(name: &str, containers: &[&str]) -> ServiceArg {
        ServiceArg {
            service: name.to_owned(),
            containers: containers.iter().map(|value| (*value).to_owned()).collect(),
        }
    }

    #[test]
    fn ports_the_oracle_grouping_cases() {
        let cases = [
            (Vec::<&str>::new(), vec![]),
            (vec!["web"], vec![service("web", &[])]),
            (vec!["web/abc123"], vec![service("web", &["abc123"])]),
            (
                vec!["web/abc123", "web/def456"],
                vec![service("web", &["abc123", "def456"])],
            ),
            (
                vec!["web/abc123", "api", " web/def456  ", "db/xyz789 "],
                vec![
                    service("web", &["abc123", "def456"]),
                    service("api", &[]),
                    service("db", &["xyz789"]),
                ],
            ),
            (vec!["web", "web/abc123"], vec![service("web", &[])]),
            (
                vec!["web/abc123", "web/def456", "web"],
                vec![service("web", &[])],
            ),
            (
                vec!["web/abc123", "web", "web/def456"],
                vec![service("web", &[])],
            ),
            (
                vec!["db", "api", "web"],
                vec![service("db", &[]), service("api", &[]), service("web", &[])],
            ),
        ];

        for (input, expected) in cases {
            assert_eq!(parse_service_args(input).unwrap(), expected);
        }
    }

    #[test]
    fn ports_the_oracle_errors() {
        let cases = [
            ("", "empty service argument"),
            ("   ", "empty service argument"),
            (
                "/abc123",
                "invalid service argument '/abc123': service name is empty",
            ),
            (
                "web/",
                "invalid service argument 'web/': container name or ID is empty",
            ),
        ];

        for (input, expected) in cases {
            assert_eq!(
                parse_service_args([input]).unwrap_err().to_string(),
                expected
            );
        }
    }

    #[test]
    fn only_the_first_slash_separates_service_and_container() {
        assert_eq!(
            parse_service_args(["web/container/path"]).unwrap(),
            [service("web", &["container/path"])]
        );
    }
}
