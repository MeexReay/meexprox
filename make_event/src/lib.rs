
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
    let mut new_args = Vec::new();
    let mut new_inits = Vec::new();
    let mut has_cancelled_field = false;

    if let Data::Struct(data) = input.data {
        if let Fields::Named(fields) = data.fields {
            for field in fields.named.iter() {
                let field_name = &field.ident;
                let field_ty = &field.ty;

                if field_name.as_ref().map(|name| name == "cancelled").unwrap_or(false) {
                    has_cancelled_field = true;
                } else {
                    new_args.push(quote! { #field_name: #field_ty });
                    new_inits.push(quote! { #field_name });
                }

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

    let cancel_methods = if has_cancelled_field {
        quote! {
            fn cancel(&mut self) {
                self.cancelled = true;
            }

            fn is_cancelled(&self) -> bool {
                self.cancelled
            }
        }
    } else {
        quote! {}
    };

    let new_method = if has_cancelled_field {
        quote! {
            pub fn new(#(#new_args),*) -> Self {
                Self {
                    #(#new_inits),*,
                    cancelled: false,
                }
            }
        }
    } else {
        quote! {
            pub fn new(#(#new_args),*) -> Self {
                Self {
                    #(#new_inits),*
                }
            }
        }
    };

    let expanded = quote! {
        impl #struct_name {
            #(#getters)*
            #(#setters)*
            #new_method
        }

        impl Event for #struct_name {
            #cancel_methods

            fn name(&self) -> String {
                #event_name.to_string()
            }
        }
    };

    TokenStream::from(expanded)
}
