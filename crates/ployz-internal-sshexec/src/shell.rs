/// Quotes one string so a POSIX shell reads it as exactly one token.
#[must_use]
pub fn quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_owned();
    }

    if value.bytes().all(is_unquoted_byte) {
        return value.to_owned();
    }

    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('\'');
    quoted.push_str(&value.replace('\'', "'\"'\"'"));
    quoted.push('\'');
    quoted
}

/// Quotes command arguments and joins them with one ASCII space.
#[must_use]
pub fn quote_command<I, S>(arguments: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    arguments
        .into_iter()
        .map(|argument| quote(argument.as_ref()))
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_unquoted_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'_' | b'@' | b'%' | b'+' | b'=' | b':' | b',' | b'.' | b'/' | b'-'
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_matches_oracle_token_rules() {
        let cases = [
            ("", "''"),
            ("plain", "plain"),
            ("under_score-1/@%+=:,.", "under_score-1/@%+=:,."),
            ("two words", "'two words'"),
            ("it's", "'it'\"'\"'s'"),
            ("é", "'é'"),
        ];

        for (input, expected) in cases {
            assert_eq!(quote(input), expected);
        }
    }

    #[test]
    fn command_quotes_each_argument_and_joins_with_spaces() {
        assert_eq!(
            quote_command(["bash", "-c", "printf '%s' hello world"]),
            "bash -c 'printf '\"'\"'%s'\"'\"' hello world'"
        );
    }
}
