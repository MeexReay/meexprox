
use proc_macro::TokenStream;
use quote::{quote, format_ident};
use syn::{parse_macro_input, DeriveInput, Data, Fields};

#[proc_macro_derive(MakeEvent, attributes(MakeEvent, setter))]
pub fn make_event_derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let struct_name = &input.ident;
    let event_name = input.attrs.iter()
        .find_map(|attr| {
            if attr.path().is_ident("MakeEvent") {
                attr.parse_args::<syn::LitStr>().ok()
            } else {
                None
            }
        })
        .expect("Expected MakeEvent attribute with a name argument")
        .value();

    let mut getters = Vec::new();
    let mut setters = Vec::new();

    if let Data::Struct(data) = input.data {
        if let Fields::Named(fields) = data.fields {
            for field in fields.named.iter() {
                let field_name = &field.ident;
                let field_ty = &field.ty;
                getters.push(quote! {
                    pub fn #field_name(&self) -> &#field_ty {
                        &self.#field_name
                    }
                });
                if field.attrs.iter().any(|attr| attr.path().is_ident("setter")) {
                    let setter_name = format_ident!("set_{}", field_name.as_ref().unwrap());
                    setters.push(quote! {
                        pub fn #setter_name(&mut self, value: #field_ty) {
                            self.#field_name = value;
                        }
                    });
                }
            }
        }
    } else {
        panic!("MakeEvent can only be derived for structs with named fields");
    }

    let expanded = quote! {
        impl #struct_name {
            #(#getters)*
            #(#setters)*
        }

        impl Event for #struct_name {
            fn cancel(&mut self) {
                self.cancelled = true;
            }

            fn is_cancelled(&self) -> bool {
                self.cancelled
            }

            fn name(&self) -> String {
                #event_name.to_string()
            }
        }
    };

    TokenStream::from(expanded)
}