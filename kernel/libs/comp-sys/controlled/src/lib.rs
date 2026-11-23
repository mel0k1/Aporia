use quote::quote;

#[proc_macro_attribute]
pub fn controlled(
    attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let attr = attr.to_string();
    if !attr.is_empty() {
        panic!("controlled cannot accept inner tokens.")
    }
    let mut tokens: proc_macro::TokenStream = quote!(
        #[component_access_control::controlled]
    )
    .into();
    tokens.extend(item);
    tokens
}

#[proc_macro_attribute]
pub fn uncontrolled(
    attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let attr = attr.to_string();
    if !attr.is_empty() {
        panic!("uncontrolled cannot accept inner tokens.")
    }
    let mut tokens: proc_macro::TokenStream = quote!(
        #[component_access_control::uncontrolled]
    )
    .into();
    tokens.extend(item);
    tokens
}
