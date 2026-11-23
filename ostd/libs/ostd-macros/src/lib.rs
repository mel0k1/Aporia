#![feature(proc_macro_diagnostic)]

use proc_macro::{Diagnostic, Level, Span, TokenStream};
use quote::quote;
use rand::{Rng, distr::Alphanumeric};
use syn::{Expr, Ident, ItemFn, parse_macro_input};

#[proc_macro_attribute]
pub fn main(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let main_fn = parse_macro_input!(item as ItemFn);
    let main_fn_name = &main_fn.sig.ident;
    let body = ostd_main_body(main_fn_name);

    quote!(
        #[cfg(not(ktest))]
        #[unsafe(no_mangle)]
        extern "Rust" fn __ostd_main() -> ! {
            #body
        }

        #[expect(unused)]
        #main_fn
    )
    .into()
}

#[doc(hidden)]
#[proc_macro_attribute]
pub fn test_main(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let main_fn = parse_macro_input!(item as ItemFn);
    let main_fn_name = &main_fn.sig.ident;
    let body = ostd_main_body(main_fn_name);

    quote!(
        #[unsafe(no_mangle)]
        extern "Rust" fn __ostd_main() -> ! {
            #body
        }

        #main_fn
    )
    .into()
}

fn ostd_main_body(main_fn_name: &Ident) -> proc_macro2::TokenStream {
    quote! {
        let _: () = #main_fn_name();

        ::ostd::task::Task::yield_now();

        ::core::assert!(::ostd::task::Task::current().is_none());
        ::ostd::power::poweroff(::ostd::power::ExitCode::Success);
    }
}

#[proc_macro_attribute]
pub fn global_frame_allocator(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as syn::ItemStatic);
    let static_name = &item.ident;

    quote!(
        #[unsafe(no_mangle)]
        static __GLOBAL_FRAME_ALLOCATOR_REF: &'static dyn ::ostd::mm::frame::GlobalFrameAllocator =
            &#static_name;

        #item
    )
    .into()
}

#[proc_macro_attribute]
pub fn global_heap_allocator(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as syn::ItemStatic);
    let static_name = &item.ident;

    quote!(
        #[unsafe(no_mangle)]
        static __GLOBAL_HEAP_ALLOCATOR_REF: &'static dyn ::ostd::mm::heap::GlobalHeapAllocator =
            &#static_name;

        #item
    )
    .into()
}

#[proc_macro_attribute]
pub fn global_heap_allocator_slot_map(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as ItemFn);
    let fn_name = &item.sig.ident;

    assert!(
        item.sig.constness.is_some(),
        "the annotated function must be `const`"
    );

    quote!(
        #[unsafe(no_mangle)]
        const extern "Rust" fn __global_heap_slot_info_from_layout(
            layout: ::core::alloc::Layout,
        ) -> ::core::option::Option<::ostd::mm::heap::SlotInfo> {
            #fn_name(layout)
        }

        #item
    )
    .into()
}

#[proc_macro_attribute]
pub fn early_cmdline_parser(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as ItemFn);
    let fn_name = &item.sig.ident;

    assert!(
        item.sig.constness.is_some(),
        "the annotated function must be `const`"
    );

    quote!(
        #[unsafe(no_mangle)]
        const extern "Rust" fn __early_cmdline_parser(
            cmdline: &str,
        ) -> ::ostd::boot::EarlyCmdline {
            #fn_name(cmdline)
        }

        #item
    )
    .into()
}

#[proc_macro_attribute]
pub fn panic_handler(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let handler_fn = parse_macro_input!(item as ItemFn);
    let handler_fn_name = &handler_fn.sig.ident;

    quote!(
        #[cfg(not(ktest))]
        #[unsafe(no_mangle)]
        extern "Rust" fn __ostd_panic_handler(info: &::core::panic::PanicInfo) -> ! {
            #handler_fn_name(info);
        }

        #[expect(unused)]
        #handler_fn
    )
    .into()
}

#[doc(hidden)]
#[proc_macro_attribute]
pub fn test_panic_handler(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let handler_fn = parse_macro_input!(item as ItemFn);
    let handler_fn_name = &handler_fn.sig.ident;

    quote!(
        #[unsafe(no_mangle)]
        extern "Rust" fn __ostd_panic_handler(info: &::core::panic::PanicInfo) -> ! {
            #handler_fn_name(info);
        }

        #handler_fn
    )
    .into()
}

#[proc_macro_attribute]
pub fn ktest(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);
    assert!(
        input.sig.inputs.is_empty(),
        "test functions should have no arguments"
    );
    assert!(
        matches!(input.sig.output, syn::ReturnType::Default),
        "test functions should return `()`"
    );

    let fn_id: String = rand::rng()
        .sample_iter(&Alphanumeric)
        .take(8)
        .map(char::from)
        .collect();

    let fn_name = &input.sig.ident;
    let fn_ktest_item_name = Ident::new(
        &format!("{}_ktest_item_{}", input.sig.ident, fn_id),
        proc_macro2::Span::call_site(),
    );

    emit_redundant_test_prefix_warnings(attr, &input.sig.ident);

    let panic_expectation_tokens = generate_panic_expectation_tokens(&input.attrs);

    let package_name = std::env::var("CARGO_PKG_NAME").unwrap();
    let span = proc_macro2::Span::call_site();
    let line = span.start().line;
    let col = span.start().column;

    let register_ktest_item = if package_name.as_str() == "ostd" {
        quote! {
            #[cfg(ktest)]
            #[used]
            #[unsafe(link_section = ".ktest_array")]
            static #fn_ktest_item_name: ::ostd_test::KtestItem = ::ostd_test::KtestItem::new(
                #fn_name,
                #panic_expectation_tokens,
                ::ostd_test::KtestItemInfo {
                    module_path: ::core::module_path!(),
                    fn_name: ::core::stringify!(#fn_name),
                    package: #package_name,
                    source: ::core::file!(),
                    line: #line,
                    col: #col,
                },
            );
        }
    } else {
        quote! {
            #[cfg(ktest)]
            #[used]
            #[unsafe(link_section = ".ktest_array")]
            static #fn_ktest_item_name: ::ostd::ktest::KtestItem = ::ostd::ktest::KtestItem::new(
                #fn_name,
                #panic_expectation_tokens,
                ::ostd::ktest::KtestItemInfo {
                    module_path: ::core::module_path!(),
                    fn_name: ::core::stringify!(#fn_name),
                    package: #package_name,
                    source: ::core::file!(),
                    line: #line,
                    col: #col,
                },
            );
        }
    };

    let output = quote! {
        #input

        #register_ktest_item
    };

    TokenStream::from(output)
}

fn emit_redundant_test_prefix_warnings(attr: TokenStream, ident: &Ident) {
    let should_have_test_prefix = if !attr.is_empty() {
        assert_eq!(
            attr.to_string(),
            "expect_redundant_test_prefix",
            "unknown arguments"
        );
        true
    } else {
        false
    };

    fn should_deny_warnings(rustflags: &str) -> bool {
        let options = rustflags.split_whitespace().collect::<Vec<_>>();

        if options.is_empty() {
            return false;
        }

        if options.contains(&"-Dwarnings") {
            return true;
        }

        for i in 0..options.len() - 1 {
            if (options[i] == "--deny" || options[i] == "-D") && options[i + 1] == "warnings" {
                return true;
            }
        }

        false
    }

    let lint_level = if let Ok(rustflags) = std::env::var("RUSTFLAGS")
        && should_deny_warnings(rustflags.as_str())
    {
        Level::Error
    } else {
        Level::Warning
    };

    let test_prefix_stripped = ident
        .to_string()
        .strip_prefix("test_")
        .map(ToOwned::to_owned);

    if !should_have_test_prefix && let Some(name_to_suggest) = test_prefix_stripped {
        Diagnostic::spanned(
            ident.span().unwrap(),
            lint_level,
            "redundant `test_` prefix in test function name",
        )
        .span_help(
            ident.span().unwrap(),
            format!(
                "consider removing the `test_` prefix: `{}`",
                name_to_suggest
            ),
        )
        .span_help(
            Span::call_site(),
            "consider allowing the lint: `#[ktest(expect_redundant_test_prefix)]`",
        )
        .help(
            "for further information visit \
             https://rust-lang.github.io/rust-clippy/master/index.html#redundant_test_prefix",
        )
        .emit()
    } else if should_have_test_prefix && test_prefix_stripped.is_none() {
        Diagnostic::spanned(
            ident.span().unwrap(),
            lint_level,
            "no redundant `test_` prefix in test function name",
        )
        .span_help(
            Span::call_site(),
            "consider removing the expectation: `#[ktest]`",
        )
        .emit();
    };
}

fn generate_panic_expectation_tokens(attrs: &[syn::Attribute]) -> proc_macro2::TokenStream {
    fn is_should_panic_attr(attr: &syn::Attribute) -> bool {
        attr.path()
            .segments
            .iter()
            .any(|segment| segment.ident == "should_panic")
    }

    let mut attr_iter = attrs.iter();
    let Some(should_panic_attr) = attr_iter.find(|&attr| is_should_panic_attr(attr)) else {
        let tokens = quote! { (false, None) };
        return tokens;
    };
    assert!(
        !attr_iter.any(is_should_panic_attr),
        "multiple `should_panic` attributes"
    );

    match &should_panic_attr.meta {
        syn::Meta::List(list) => {
            if let Ok(expected_assign) = syn::parse2::<syn::ExprAssign>(list.tokens.clone())
                && let Expr::Lit(lit) = *expected_assign.right
                && let syn::Lit::Str(expectation) = lit.lit
            {
                let tokens = quote! { (true, Some(#expectation)) };
                return tokens;
            }
        }
        syn::Meta::Path(_) => {
            let tokens = quote! { (true, None) };
            return tokens;
        }
        _ => (),
    }

    panic!(
        "`should_panic` attributes should only have zero or one `expected` argument, \
         with the format of `expected = \"<panic message>\"`"
    );
}
