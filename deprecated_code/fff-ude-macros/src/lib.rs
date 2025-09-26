use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use syn::Result;

#[proc_macro_attribute]
pub fn function(attr: TokenStream, item: TokenStream) -> TokenStream {
    fn inner(_attr: TokenStream, _item: TokenStream) -> Result<TokenStream2> {
        // let fn_attr: FunctionAttr = syn::parse(attr)?;
        // let user_fn: UserFunctionAttr = syn::parse(item.clone())?;

        // let mut tokens: TokenStream2 = item.into();
        // for attr in fn_attr.expand() {
        //     tokens.extend(attr.generate_function_descriptor(&user_fn)?);
        // }
        // Ok(tokens)
        todo!()
    }
    match inner(attr, item) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}
