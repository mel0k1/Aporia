#![deny(unsafe_code)]

mod init_comp;
mod priority;

use init_comp::ComponentInitFunction;
use proc_macro::TokenStream;
use quote::quote;
use syn::parse_macro_input;

pub(crate) const COMPONENT_FILE_NAME: &str = "Components.toml";

#[proc_macro_attribute]
pub fn init_component(args: TokenStream, input: TokenStream) -> TokenStream {
    let stage = match args.to_string().as_str() {
        "" | "bootstrap" => quote! { Bootstrap },
        "kthread" => quote! { Kthread },
        "process" => quote! { Process },
        _ => panic!("Invalid argument for init_component"),
    };
    let function = parse_macro_input!(input as ComponentInitFunction);
    let function_name = &function.function_name;
    quote! {
        #function

        component::submit!(component::ComponentRegistry::new(
            component::InitStage::#stage,
            &#function_name,
            file!())
        );
    }
    .into()
}

#[proc_macro]
pub fn parse_metadata(_: TokenStream) -> TokenStream {
    let out = priority::component_generate();
    let path = priority::get_component_toml_path();
    quote! {
        {
            include_str!(#path);
            extern crate alloc;
            alloc::vec![
                #(component::ComponentInfo::new #out),*
            ]
        }
    }
    .into()
}
