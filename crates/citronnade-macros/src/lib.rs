//! `#[citronnade::main]` and `shader!`, written without `syn` or `quote` to
//! keep the dependency tree empty.

use proc_macro::{Delimiter, Group, Ident, Literal, Punct, Spacing, Span, TokenStream, TokenTree};

/// Marks the program's entry point.
///
/// ```ignore
/// #[citronnade::main]
/// fn main() { ... }
///
/// #[citronnade::main(stack_size = 512 * 1024, linear_heap_size = 16 << 20)]
/// fn main(sys: citronnade::System) -> citronnade::Result<()> { ... }
/// ```
///
/// `main` may return `()`, `i32` or a `Result` whose error is `Debug`;
/// an error is logged and the program exits with code 1. If it takes a
/// parameter, it receives the `System` (all services, each taken once).
///
/// Options (all optional): `stack_size` (bytes, default 256 KiB),
/// `heap_size` and `linear_heap_size` (bytes, default: split the free memory
/// as libctru does).
#[proc_macro_attribute]
pub fn main(attr: TokenStream, item: TokenStream) -> TokenStream {
    let options = match parse_options(attr) {
        Ok(o) => o,
        Err(msg) => return compile_error(&msg),
    };
    let (name, takes_system) = match parse_fn(item.clone()) {
        Ok(v) => v,
        Err(msg) => return compile_error(&msg),
    };
    let call = if takes_system {
        format!("{name}(::citronnade::System::__take())")
    } else {
        format!("{name}()")
    };
    let shim = format!(
        r#"
        #[doc(hidden)]
        #[unsafe(export_name = "__citronnade_main")]
        fn __citronnade_main_shim() -> i32 {{
            ::citronnade::__private::Termination::report({call})
        }}
        #[doc(hidden)]
        #[unsafe(no_mangle)]
        #[used]
        static __CITRONNADE_STACK_SIZE: u32 = ({stack}) as u32;
        #[doc(hidden)]
        #[unsafe(no_mangle)]
        #[used]
        static __CITRONNADE_HEAP_SIZE: u32 = ({heap}) as u32;
        #[doc(hidden)]
        #[unsafe(no_mangle)]
        #[used]
        static __CITRONNADE_LINEAR_HEAP_SIZE: u32 = ({linear}) as u32;
        "#,
        stack = options.stack_size.as_deref().unwrap_or("0"),
        heap = options.heap_size.as_deref().unwrap_or("0"),
        linear = options.linear_heap_size.as_deref().unwrap_or("0"),
    );
    let mut out = item;
    out.extend(shim.parse::<TokenStream>().expect("generated code parses"));
    out
}

#[derive(Default)]
struct Options {
    stack_size: Option<String>,
    heap_size: Option<String>,
    linear_heap_size: Option<String>,
}

fn parse_options(attr: TokenStream) -> Result<Options, String> {
    let mut options = Options::default();
    let tokens: Vec<TokenTree> = attr.into_iter().collect();
    let mut i = 0;
    while i < tokens.len() {
        let key = match &tokens[i] {
            TokenTree::Ident(id) => id.to_string(),
            other => return Err(format!("expected an option name, found `{other}`")),
        };
        i += 1;
        match tokens.get(i) {
            Some(TokenTree::Punct(p)) if p.as_char() == '=' => i += 1,
            _ => return Err(format!("expected `=` after `{key}`")),
        }
        let mut value = String::new();
        while i < tokens.len() {
            if let TokenTree::Punct(p) = &tokens[i] {
                if p.as_char() == ',' {
                    i += 1;
                    break;
                }
            }
            value.push_str(&tokens[i].to_string());
            value.push(' ');
            i += 1;
        }
        if value.trim().is_empty() {
            return Err(format!("missing value for `{key}`"));
        }
        let slot = match key.as_str() {
            "stack_size" => &mut options.stack_size,
            "heap_size" => &mut options.heap_size,
            "linear_heap_size" => &mut options.linear_heap_size,
            _ => {
                return Err(format!(
                    "unknown option `{key}` (expected stack_size, heap_size or linear_heap_size)"
                ));
            }
        };
        *slot = Some(value);
    }
    Ok(options)
}

/// Finds the function's name and whether it takes a parameter.
fn parse_fn(item: TokenStream) -> Result<(String, bool), String> {
    let mut iter = item.into_iter().peekable();
    while let Some(tt) = iter.next() {
        if let TokenTree::Ident(id) = &tt {
            if id.to_string() == "fn" {
                let name = match iter.next() {
                    Some(TokenTree::Ident(n)) => n.to_string(),
                    _ => return Err("expected a function name".into()),
                };
                // Skip generics, if any, up to the parameter list.
                for tt in iter.by_ref() {
                    if let TokenTree::Group(g) = &tt {
                        if g.delimiter() == Delimiter::Parenthesis {
                            let takes = !g.stream().is_empty();
                            return Ok((name, takes));
                        }
                    }
                }
                return Err("expected a parameter list".into());
            }
        }
    }
    Err("#[citronnade::main] must be placed on a function".into())
}

fn compile_error(msg: &str) -> TokenStream {
    format!("::core::compile_error!({msg:?});").parse().unwrap()
}

/// Assembles a PICA200 shader at compile time into SHBIN bytes
/// (`&'static [u8]`), reporting assembly errors with their line.
///
/// ```ignore
/// let bin: &[u8] = shader!(r#"
///     .fvec projection[4]
///     .out outpos position
///     .proc main
///         dp4 outpos.x, projection[0], v0
///         ...
///         end
///     .end
/// "#);
/// let bin = shader!(file = "shaders/sprite.pica"); // relative to Cargo.toml
/// ```
#[proc_macro]
pub fn shader(input: TokenStream) -> TokenStream {
    let tokens: Vec<TokenTree> = input.into_iter().collect();
    let (source, span, tracked) = match tokens.as_slice() {
        [TokenTree::Literal(lit)] => match unquote(&lit.to_string()) {
            Some(s) => (s, lit.span(), None),
            None => return compile_error_at("shader! expects a string literal", lit.span()),
        },
        [TokenTree::Ident(id), TokenTree::Punct(eq), TokenTree::Literal(lit)] if id.to_string() == "file" && eq.as_char() == '=' => {
            let Some(rel) = unquote(&lit.to_string()) else {
                return compile_error_at("shader!(file = ...) expects a string literal", lit.span());
            };
            let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
            let path = std::path::Path::new(&dir).join(&rel);
            match std::fs::read_to_string(&path) {
                Ok(s) => (s, lit.span(), Some(path.display().to_string())),
                Err(e) => return compile_error_at(&format!("cannot read {}: {e}", path.display()), lit.span()),
            }
        }
        _ => return compile_error_at("shader! expects a string literal or `file = \"path\"`", Span::call_site()),
    };
    let program = match citronnade_shader::assemble(&source) {
        Ok(p) => p,
        Err(e) => {
            let where_ = tracked.as_deref().map(|p| format!("{p}: ")).unwrap_or_default();
            return compile_error_at(&format!("shader: {where_}line {}: {}", e.line, e.message), span);
        }
    };
    let bytes = Literal::byte_string(&program.to_shbin());
    let mut out = TokenStream::new();
    if let Some(path) = tracked {
        // Rebuild when the file changes.
        out.extend(format!("const _: &[u8] = include_bytes!({path:?});").parse::<TokenStream>().unwrap());
    }
    out.extend([TokenTree::Literal(bytes)]);
    TokenStream::from(TokenTree::Group(Group::new(Delimiter::Brace, out)))
}

fn compile_error_at(msg: &str, span: Span) -> TokenStream {
    let mut msg_lit = Literal::string(msg);
    msg_lit.set_span(span);
    let mut name = Ident::new("compile_error", span);
    name.set_span(span);
    let mut bang = Punct::new('!', Spacing::Alone);
    bang.set_span(span);
    let mut group = Group::new(Delimiter::Parenthesis, TokenStream::from(TokenTree::Literal(msg_lit)));
    group.set_span(span);
    let mut semi = Punct::new(';', Spacing::Alone);
    semi.set_span(span);
    let call: TokenStream = [TokenTree::Ident(name), TokenTree::Punct(bang), TokenTree::Group(group)].into_iter().collect();
    let mut braced = Group::new(Delimiter::Brace, call);
    braced.set_span(span);
    TokenStream::from(TokenTree::Group(braced))
}

/// The value of a string literal's source text (plain or raw).
fn unquote(lit: &str) -> Option<String> {
    if let Some(rest) = lit.strip_prefix('r') {
        let hashes = rest.chars().take_while(|&c| c == '#').count();
        let body = rest.get(hashes..)?.strip_prefix('"')?;
        let body = body.strip_suffix(&"#".repeat(hashes))?.strip_suffix('"')?;
        return Some(body.to_string());
    }
    let body = lit.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next()? {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            '0' => out.push('\0'),
            '\\' => out.push('\\'),
            '"' => out.push('"'),
            '\'' => out.push('\''),
            'x' => {
                let h: String = [chars.next()?, chars.next()?].iter().collect();
                out.push(u8::from_str_radix(&h, 16).ok()? as char);
            }
            'u' => {
                chars.next()?; // {
                let mut h = String::new();
                for c in chars.by_ref() {
                    if c == '}' {
                        break;
                    }
                    h.push(c);
                }
                out.push(char::from_u32(u32::from_str_radix(&h, 16).ok()?)?);
            }
            '\n' => {
                while chars.peek().is_some_and(|c| c.is_whitespace()) {
                    chars.next();
                }
            }
            _ => return None,
        }
    }
    Some(out)
}
