#![doc = include_str!("../README.md")]

use proc_macro::TokenStream;

mod pod_derive;
mod pod_union;

#[proc_macro_attribute]
pub fn derive(attrs: TokenStream, input: TokenStream) -> TokenStream {
    pod_derive::expand_derive(attrs, input)
}

#[proc_macro_attribute]
pub fn pod_union(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(item as syn::DeriveInput);
    pod_union::expand_pod_union(input).into()
}
