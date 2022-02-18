use crate::parser::Node;
use crate::scanner::Range;

type NodeIter<'a> = std::iter::Peekable<std::slice::Iter<'a, Node>>;

#[derive(Debug)]
pub enum RenderError {
    DuplicateParamName(String, Range),
}

pub fn render(iter: &mut NodeIter) -> Result<String, RenderError> {
    let (builder_lines, imports, typed_params) = render_lines(iter)?;

    let import_lines = imports
        .iter()
        .map(|details| format!("import {}", details))
        .collect::<Vec<_>>()
        .join("\n");

    let params_string = typed_params
        .iter()
        .map(|(param_name, type_name)| format!("{} {}: {}", param_name, param_name, type_name))
        .collect::<Vec<_>>()
        .join(", ");

    let args_string = typed_params
        .iter()
        .map(|(param_name, _)| format!("{}: {}", param_name, param_name))
        .collect::<Vec<_>>()
        .join(", ");

    let output = format!(
        r#"import gleam/string_builder.{{StringBuilder}}
import gleam/list

{}

pub fn render_builder({}) -> StringBuilder {{
    let builder = string_builder.from_string("")
{}
    builder
}}

pub fn render({}) -> String {{
    string_builder.to_string(render_builder({}))
}}
"#,
        import_lines, params_string, builder_lines, params_string, args_string
    );

    Ok(output)
}

type RenderDetails = (String, Vec<String>, Vec<(String, String)>);

fn render_lines(iter: &mut NodeIter) -> Result<RenderDetails, RenderError> {
    let mut builder_lines = String::new();
    let mut imports = vec![];

    // Use a Vec<(String, String)> instead of a HashMap to maintain order which gives the users
    // some control, though parameters are labelled and can be called in any order. Some kind of
    // order is required to keep the tests passing as it seems to be non-determinate in a HashMap
    let mut typed_params = Vec::new();

    loop {
        match iter.peek() {
            Some(Node::Text(text)) => {
                iter.next();
                builder_lines.push_str(&format!(
                    "    let builder = string_builder.append(builder, \"{}\")\n",
                    text.replace("\"", "\\\"")
                ));
            }
            Some(Node::Identifier(name)) => {
                iter.next();
                builder_lines.push_str(&format!(
                    "    let builder = string_builder.append(builder, {})\n",
                    name
                ));
            }
            Some(Node::Builder(name)) => {
                iter.next();
                builder_lines.push_str(&format!(
                    "    let builder = string_builder.append_builder(builder, {})\n",
                    name
                ));
            }
            Some(Node::Import(import_details)) => {
                iter.next();
                imports.push(import_details.clone());
            }
            Some(Node::With((identifier, range), type_)) => {
                iter.next();

                if typed_params.iter().any(|(name, _)| name == identifier) {
                    return Err(RenderError::DuplicateParamName(
                        identifier.clone(),
                        range.clone(),
                    ));
                }

                typed_params.push((identifier.clone(), type_.clone()));
            }
            Some(Node::If(identifier_name, if_nodes, else_nodes)) => {
                iter.next();
                let (if_lines, _, _) = render_lines(&mut if_nodes.iter().peekable())?;
                let (else_lines, _, _) = render_lines(&mut else_nodes.iter().peekable())?;
                builder_lines.push_str(&format!(
                    r#"    let builder = case {} {{
        True -> {{
            {}
            builder
        }}
        False -> {{
            {}
            builder
        }}
}}
"#,
                    identifier_name, if_lines, else_lines
                ));
            }
            Some(Node::For(entry_identifier, entry_type, list_identifier, loop_nodes)) => {
                iter.next();

                let entry_type = entry_type
                    .as_ref()
                    .map(|value| format!(": {}", value))
                    .unwrap_or_else(|| "".to_string());

                let (loop_lines, _, _) = render_lines(&mut loop_nodes.iter().peekable())?;
                builder_lines.push_str(&format!(
                    r#"    let builder = list.fold({}, builder, fn(builder, {}{}) {{
        {}
        builder
}})
"#,
                    list_identifier, entry_identifier, entry_type, loop_lines
                ));
            }
            None => break,
        }
    }

    Ok((builder_lines, imports, typed_params))
}

#[cfg(test)]
mod test {
    use super::*;

    use crate::parser::{self, ParserError};
    use crate::scanner::{self, ScanError};

    #[derive(Debug)]
    pub enum Error {
        Scan(ScanError),
        Parse(ParserError),
        Render(RenderError),
    }

    fn format_result(result: Result<String, Error>) -> String {
        match result {
            Ok(value) => value,
            Err(err) => format!("{:?}", err),
        }
    }

    #[macro_export]
    macro_rules! assert_render {
        ($text:expr $(,)?) => {{
            let _ = env_logger::try_init();
            let result = scanner::scan($text)
                .map_err(|err| Error::Scan(err))
                .and_then(|tokens| {
                    parser::parse(&mut tokens.iter().peekable()).map_err(|err| Error::Parse(err))
                })
                .and_then(|ast| {
                    render(&mut ast.iter().peekable()).map_err(|err| Error::Render(err))
                });
            insta::assert_snapshot!(insta::internals::AutoName, format_result(result), $text);
        }};
    }

    // Render

    #[test]
    fn test_render_pure_text() {
        assert_render!("Hello name, good to meet you");
    }

    #[test]
    fn test_render_identifier() {
        assert_render!(
            "{> with name as String
Hello {{ name }}, good to meet you"
        );
    }

    #[test]
    fn test_render_two_identifiers() {
        assert_render!(
            "{> with name as String
{> with adjective as String
Hello {{ name }}, {{ adjective }} to meet you"
        );
    }

    #[test]
    fn test_repeated_identifier_usage() {
        assert_render!(
            "{> with name as String
{{ name }} usage, {{ name }} usage"
        );
    }

    #[test]
    fn test_render_if_statement() {
        assert_render!(
            "{> with is_user as Bool
Hello {% if is_user %}User{% endif %}"
        );
    }

    #[test]
    fn test_render_empty_if_statement() {
        assert_render!(
            "{> with is_user as Bool
Hello {% if is_user %}{% endif %}"
        );
    }

    #[test]
    fn test_render_if_else_statement() {
        assert_render!(
            "{> with is_user as Bool
Hello {% if is_user %}User{% else %}Unknown{% endif %}"
        );
    }

    #[test]
    fn test_render_nested_if_statements() {
        assert_render!(
            "{> with is_user as Bool
{> with is_admin as Bool
Hello {% if is_user %}{% if is_admin %}Admin{% else %}User{% endif %}{% endif %}"
        );
    }

    #[test]
    fn test_render_for_loop() {
        assert_render!(
            "{> with list as List(String)
Hello,{% for item in list %} to {{ item }} and {% endfor %} everyone else"
        );
    }

    #[test]
    fn test_render_for_as_loop() {
        assert_render!(
            "{> with list as List(Item)
Hello,{% for item as Item in list %} to {{ item }} and {% endfor %} everyone else"
        );
    }

    #[test]
    fn test_render_dot_access() {
        assert_render!(
            "{> with user as MyUser
Hello{% if user.is_admin %} Admin{% endif %}"
        );
    }

    #[test]
    fn test_render_import() {
        assert_render!("{> import user.{User}\n{> with name as String\n{{ name }}");
    }

    #[test]
    fn test_render_with() {
        assert_render!("{> with user as User\n{{ user }}");
    }

    #[test]
    fn test_render_import_and_with() {
        assert_render!("{> import user.{User}\n{> with user as User\n{{ user }}");
    }

    #[test]
    fn test_render_multiline() {
        assert_render!(
            r#"{> with my_list as List(String)
<ul>
{% for entry in my_list %}
    <li>{{ entry }}</li>
{% endfor %}
</ul>"#
        );
    }

    #[test]
    fn test_render_quotes() {
        assert_render!(
            r#"{> with name as String
<div class="my-class">{{ name }}</div>"#
        );
    }

    #[test]
    fn test_render_builder_block() {
        assert_render!(
            "{> with name as StringBuilder
Hello {[ name ]}, good to meet you"
        );
    }
}
