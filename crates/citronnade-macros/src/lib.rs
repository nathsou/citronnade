//! `#[citronnade::main]`, written without `syn` or `quote` to keep the
//! dependency tree empty.

use proc_macro::{Delimiter, TokenStream, TokenTree};

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
