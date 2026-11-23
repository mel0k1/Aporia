#![doc = include_str!("../README.md")]

use proc_macro::TokenStream;
use quote::quote;
use syn::{
    Attribute, Data, DataStruct, DeriveInput, Fields, Ident, Token, parse_macro_input,
    punctuated::Punctuated, spanned::Spanned,
};

fn has_repr_c(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        if attr.path().is_ident("repr") {
            let result = attr.parse_args_with(Punctuated::<syn::Meta, Token![,]>::parse_terminated);

            if let Ok(list) = result {
                return list
                    .iter()
                    .any(|meta| matches!(meta, syn::Meta::Path(path) if path.is_ident("C")));
            }

            false
        } else {
            false
        }
    })
}

fn extract_repr_attrs(attrs: &[Attribute]) -> Vec<&Attribute> {
    attrs
        .iter()
        .filter(|attr| attr.path().is_ident("repr"))
        .collect()
}

#[proc_macro_attribute]
pub fn padding_struct(args: TokenStream, input: TokenStream) -> TokenStream {
    if !args.is_empty() {
        panic!("`#[padding_struct]` does not accept any arguments");
    }
    let input = parse_macro_input!(input as DeriveInput);

    let fields = match &input.data {
        Data::Struct(DataStruct {
            fields: Fields::Named(fields),
            ..
        }) => &fields.named,
        _ => panic!("`#[padding_struct]` only supports named-field structs"),
    };

    if !has_repr_c(&input.attrs) {
        panic!("`#[padding_struct]` requires `#[repr(C)]` or `#[repr(C, ...)]` on struct");
    }

    let name = &input.ident;
    let vis = &input.vis;
    let generics = &input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let ref_name = Ident::new(&format!("__{}__", name), name.span());

    let repr_attrs = extract_repr_attrs(&input.attrs);

    let ref_fields: Vec<_> = fields.iter().collect();
    let ref_struct = quote! {
        #(#repr_attrs)*
        #[allow(missing_docs)]
        #[doc(hidden)]
        struct #ref_name #impl_generics #where_clause {
            #(#ref_fields),*
        }
    };

    let padded_attrs: Vec<_> = input
        .attrs
        .iter()
        .filter(|attr| !attr.path().is_ident("padding_struct"))
        .collect();

    let mut padded_fields = Vec::new();
    let field_vec: Vec<_> = fields.iter().collect();

    for (i, field) in field_vec.iter().enumerate() {
        let field_name = &field.ident;
        let field_ty = &field.ty;
        let field_attrs = &field.attrs;
        let field_vis = &field.vis;

        padded_fields.push(quote! {
            #(#field_attrs)*
            #field_vis #field_name: #field_ty
        });

        let pad_num = i + 1;
        let pad_ident = Ident::new(&format!("__pad{}", pad_num), field.span());

        let pad_size_expr = if i == field_vec.len() - 1 {
            quote! {
                ::core::mem::size_of::<#ref_name #ty_generics>()
                    - ::core::mem::offset_of!(#ref_name #ty_generics, #field_name)
                    - ::core::mem::size_of::<#field_ty>()
            }
        } else {
            let next_field = field_vec[i + 1];
            let next_field_name = &next_field.ident;
            quote! {
                ::core::mem::offset_of!(#ref_name #ty_generics, #next_field_name)
                    - ::core::mem::offset_of!(#ref_name #ty_generics, #field_name)
                    - ::core::mem::size_of::<#field_ty>()
            }
        };

        padded_fields.push(quote! {
            #[allow(missing_docs)]
            pub #pad_ident: [u8; { #pad_size_expr }]
        });
    }

    let padded_struct = quote! {
        #(#padded_attrs)*
        #vis struct #name #impl_generics #where_clause {
            #(#padded_fields),*
        }
    };

    let size_align_check = quote! {
        const _: () = {
            const _: [(); ::core::mem::size_of::<#ref_name #ty_generics>()] =
                [(); ::core::mem::size_of::<#name #ty_generics>()];

            const _: [(); ::core::mem::align_of::<#ref_name #ty_generics>()] =
                [(); ::core::mem::align_of::<#name #ty_generics>()];
        };
    };

    let expanded = quote! {
        #ref_struct

        #padded_struct

        #size_align_check
    };

    TokenStream::from(expanded)
}
