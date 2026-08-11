use ployz_internal_version::{get_info, version};

#[allow(unexpected_cfgs)]
const INJECTED_CFG: bool = cfg!(INJECTED);

fn main() {
    let field = std::env::args().nth(1).expect("field argument");
    let info = get_info();
    let value = match field.as_str() {
        "version" => version(),
        "commit" => info.git_commit,
        "dirty" => info.git_state,
        "date" => &info.build_date,
        "built-by" => info.built_by,
        "compiler" => info.go_version,
        "platform" => &info.platform,
        "injected-cfg" => return print!("{INJECTED_CFG}"),
        "json" => return print!("{}", info.json_string()),
        "text" => return print!("{info}"),
        _ => panic!("unknown field: {field}"),
    };
    print!("{value}");
}
