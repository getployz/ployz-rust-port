use std::collections::BTreeMap;
use std::net::IpAddr;
use std::sync::Arc;

use gotmpl::{Result as TemplateResult, Template, TemplateError, Value};

#[derive(Clone, Debug)]
pub(super) struct TemplateContext {
    pub(super) name: String,
    pub(super) upstreams: BTreeMap<String, Vec<String>>,
}

pub(super) fn render_caddyfile(context: &TemplateContext, source: &str) -> Result<String, String> {
    let function_context = Arc::new(context.clone());
    let template = Template::new("Caddyfile")
        .func("upstreams", move |arguments| {
            render_upstreams(&function_context, arguments)
        })
        .parse(source)
        .map_err(|error| format!("parse config as Go template: {error}"))?;

    template
        .execute_to_string(&template_value(context))
        .map_err(|error| format!("execute template: {error}"))
}

fn template_value(context: &TemplateContext) -> Value {
    let upstreams = context
        .upstreams
        .iter()
        .map(|(name, addresses)| {
            let addresses = addresses
                .iter()
                .cloned()
                .map(Value::from)
                .collect::<Vec<_>>();
            (
                Arc::<str>::from(name.as_str()),
                Value::List(addresses.into()),
            )
        })
        .collect::<BTreeMap<_, _>>();
    Value::Map(Arc::new(BTreeMap::from([
        (Arc::<str>::from("Name"), Value::from(context.name.clone())),
        (
            Arc::<str>::from("Upstreams"),
            Value::Map(Arc::new(upstreams)),
        ),
    ])))
}

fn render_upstreams(context: &TemplateContext, arguments: &[Value]) -> TemplateResult<Value> {
    let (service_name, port) = match arguments {
        [] => (context.name.as_str(), 0),
        [Value::Int(port)] => (context.name.as_str(), *port),
        [Value::String(service_name)] => (service_name.as_ref(), 0),
        [argument] => {
            return Err(execution_error(format!(
                "upstreams function: invalid argument type: {}",
                argument.type_name()
            )));
        }
        [Value::String(service_name), Value::Int(port)] => (service_name.as_ref(), *port),
        [first, _] if !matches!(first, Value::String(_)) => {
            return Err(execution_error(
                "upstreams function: first argument must be service name (string)",
            ));
        }
        [_, _] => {
            return Err(execution_error(
                "upstreams function: second argument must be port (int)",
            ));
        }
        _ => {
            return Err(execution_error(format!(
                "upstreams function: too many arguments; expected 0-2, got {}",
                arguments.len()
            )));
        }
    };

    let rendered = context
        .upstreams
        .get(service_name)
        .into_iter()
        .flatten()
        .map(|address| with_port(address, port))
        .collect::<Vec<_>>()
        .join(" ");
    Ok(rendered.into())
}

fn execution_error(message: impl Into<String>) -> TemplateError {
    TemplateError::Exec(message.into())
}

fn with_port(address: &str, port: i64) -> String {
    if port <= 0 {
        return address.to_owned();
    }
    match address.parse::<IpAddr>() {
        Ok(IpAddr::V6(_)) => format!("[{address}]:{port}"),
        _ => format!("{address}:{port}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> TemplateContext {
        TemplateContext {
            name: "gateway".into(),
            upstreams: BTreeMap::from([
                (
                    "gateway".into(),
                    vec!["10.0.0.1".into(), "2001:db8::1".into()],
                ),
                ("web".into(), vec!["10.0.0.2".into(), "2001:db8::2".into()]),
                ("empty".into(), Vec::new()),
            ]),
        }
    }

    #[test]
    fn approved_gotmpl_seam_preserves_principal_valid_forms() {
        let cases = [
            ("{{.Name}}", "gateway"),
            ("{{upstreams}}", "10.0.0.1 2001:db8::1"),
            ("{{upstreams 8080}}", "10.0.0.1:8080 [2001:db8::1]:8080"),
            ("{{upstreams \"web\"}}", "10.0.0.2 2001:db8::2"),
            (
                "{{upstreams \"web\" 9000}}",
                "10.0.0.2:9000 [2001:db8::2]:9000",
            ),
            ("x{{upstreams \"missing\"}}y", "xy"),
            (
                "A {{- range $ip := index .Upstreams \"web\"}}https://{{$ip}};{{end}} Z",
                "Ahttps://10.0.0.2;https://2001:db8::2; Z",
            ),
        ];
        for (source, expected) in cases {
            assert_eq!(render_caddyfile(&context(), source).unwrap(), expected);
        }
    }

    #[test]
    fn approved_deviations_are_characterized() {
        assert_eq!(
            render_caddyfile(&context(), "x{{index .Upstreams \"missing\"}}y").unwrap(),
            "x<no value>y"
        );
        let parse =
            render_caddyfile(&context(), "bad {\n\treverse_proxy {{upstreams\n}").unwrap_err();
        assert!(parse.starts_with("parse config as Go template:"));
        let execution = render_caddyfile(&context(), "{{upstreams true}}").unwrap_err();
        assert!(execution.starts_with("execute template:"));
        assert!(execution.contains("upstreams function: invalid argument type: bool"));
    }
}
