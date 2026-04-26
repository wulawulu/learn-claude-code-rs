use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    Attribute, Expr, ExprLit, FnArg, Ident, ItemFn, Lit, MetaNameValue, Pat, PatIdent, PatType,
    ReturnType, Type, parse_macro_input, punctuated::Punctuated, token::Comma,
};

#[proc_macro_attribute]
pub fn tool(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args with Punctuated::<MetaNameValue, Comma>::parse_terminated);
    let item_fn = parse_macro_input!(input as ItemFn);

    match expand_tool(args, item_fn) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

fn expand_tool(
    args: Punctuated<MetaNameValue, Comma>,
    item_fn: ItemFn,
) -> syn::Result<proc_macro2::TokenStream> {
    let name = required_string_arg(&args, "name")?;
    let description = required_string_arg(&args, "description")?;

    let fn_ident = &item_fn.sig.ident;
    let wrapper_ident = format_ident!("{}Tool", to_pascal_case(&fn_ident.to_string()));
    let output = output_handling(&item_fn)?;

    if let Some(input_ty) = extract_stateful_handler_input_type(&item_fn)? {
        expand_stateful_tool(item_fn, wrapper_ident, name, description, input_ty, output)
    } else {
        expand_pure_tool(item_fn, wrapper_ident, name, description, output)
    }
}

fn required_string_arg(args: &Punctuated<MetaNameValue, Comma>, name: &str) -> syn::Result<String> {
    for arg in args {
        if arg.path.is_ident(name)
            && let Expr::Lit(ExprLit {
                lit: Lit::Str(value),
                ..
            }) = &arg.value
        {
            return Ok(value.value());
        }
    }

    Err(syn::Error::new_spanned(
        args,
        format!("missing required string argument `{name}`"),
    ))
}

struct OutputHandling {
    await_expr: proc_macro2::TokenStream,
}

fn output_handling(item_fn: &ItemFn) -> syn::Result<OutputHandling> {
    if item_fn.sig.asyncness.is_none() {
        return Err(syn::Error::new_spanned(
            item_fn.sig.fn_token,
            "tool handlers must be async functions",
        ));
    }

    match &item_fn.sig.output {
        ReturnType::Default => Err(syn::Error::new_spanned(
            &item_fn.sig.ident,
            "tool handlers must return a Display value or Result<Display>",
        )),
        ReturnType::Type(_, ty) if is_result_type(ty) => Ok(OutputHandling {
            await_expr: quote! {
                let output = call_future.await?;
                Ok(output.to_string())
            },
        }),
        ReturnType::Type(_, _) => Ok(OutputHandling {
            await_expr: quote! {
                let output = call_future.await;
                Ok(output.to_string())
            },
        }),
    }
}

fn is_result_type(ty: &Type) -> bool {
    let Type::Path(type_path) = ty else {
        return false;
    };

    let Some(segment) = type_path.path.segments.last() else {
        return false;
    };

    segment.ident == "Result"
}

fn expand_stateful_tool(
    item_fn: ItemFn,
    wrapper_ident: Ident,
    name: String,
    description: String,
    input_ty: Type,
    output: OutputHandling,
) -> syn::Result<proc_macro2::TokenStream> {
    let fn_ident = &item_fn.sig.ident;
    let await_expr = output.await_expr;

    Ok(quote! {
        #item_fn

        pub struct #wrapper_ident;

        #[async_trait::async_trait]
        impl crate::tool::Tool for #wrapper_ident {
            fn name(&self) -> &'static str {
                #name
            }

            fn description(&self) -> &'static str {
                #description
            }

            fn input_schema(&self) -> serde_json::Value {
                crate::tool::input_schema::<#input_ty>()
            }

            async fn call(
                &self,
                context: crate::tool::ToolContext,
                input: serde_json::Value,
            ) -> anyhow::Result<String> {
                let input: #input_ty = serde_json::from_value(input)?;
                let call_future = #fn_ident(context, input);
                #await_expr
            }
        }
    })
}

fn expand_pure_tool(
    mut item_fn: ItemFn,
    wrapper_ident: Ident,
    name: String,
    description: String,
    output: OutputHandling,
) -> syn::Result<proc_macro2::TokenStream> {
    let fn_ident = item_fn.sig.ident.clone();
    let input_ident = format_ident!("{}Input", to_pascal_case(&fn_ident.to_string()));
    let await_expr = output.await_expr;

    let args = item_fn
        .sig
        .inputs
        .iter()
        .map(extract_plain_arg)
        .collect::<syn::Result<Vec<_>>>()?;
    clear_parameter_attrs(&mut item_fn);
    let fields = args.iter().map(|arg| {
        let attrs = &arg.attrs;
        let ident = &arg.ident;
        let ty = &arg.ty;
        quote! {
            #(#attrs)*
            pub #ident: #ty
        }
    });
    let call_args = args.iter().map(|arg| {
        let ident = &arg.ident;
        quote! { input.#ident }
    });

    Ok(quote! {
        #item_fn

        #[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
        pub struct #input_ident {
            #(#fields,)*
        }

        pub struct #wrapper_ident;

        #[async_trait::async_trait]
        impl crate::tool::Tool for #wrapper_ident {
            fn name(&self) -> &'static str {
                #name
            }

            fn description(&self) -> &'static str {
                #description
            }

            fn input_schema(&self) -> serde_json::Value {
                crate::tool::input_schema::<#input_ident>()
            }

            async fn call(
                &self,
                _context: crate::tool::ToolContext,
                input: serde_json::Value,
            ) -> anyhow::Result<String> {
                let input: #input_ident = serde_json::from_value(input)?;
                let call_future = #fn_ident(#(#call_args),*);
                #await_expr
            }
        }
    })
}

struct PlainArg {
    attrs: Vec<Attribute>,
    ident: Ident,
    ty: Type,
}

fn extract_plain_arg(arg: &FnArg) -> syn::Result<PlainArg> {
    let FnArg::Typed(PatType { attrs, pat, ty, .. }) = arg else {
        return Err(syn::Error::new_spanned(
            arg,
            "tool handlers must not take self",
        ));
    };

    let Pat::Ident(PatIdent { ident, .. }) = pat.as_ref() else {
        return Err(syn::Error::new_spanned(
            pat,
            "plain tool handler arguments must be named like `a: i32`",
        ));
    };

    Ok(PlainArg {
        attrs: attrs.clone(),
        ident: ident.clone(),
        ty: ty.as_ref().clone(),
    })
}

fn extract_stateful_handler_input_type(item_fn: &ItemFn) -> syn::Result<Option<Type>> {
    let mut inputs = item_fn.sig.inputs.iter();
    let Some(context_arg) = inputs.next() else {
        return Ok(None);
    };
    let Some(input_arg) = inputs.next() else {
        return Ok(None);
    };

    if inputs.next().is_some() {
        return Ok(None);
    }

    if !is_type_named(context_arg, "ToolContext") {
        return Ok(None);
    }
    let input_ty = extract_arg_type(input_arg)?;

    Ok(Some(input_ty))
}

fn clear_parameter_attrs(item_fn: &mut ItemFn) {
    for arg in &mut item_fn.sig.inputs {
        if let FnArg::Typed(PatType { attrs, .. }) = arg {
            attrs.clear();
        }
    }
}

fn is_type_named(arg: &FnArg, expected: &str) -> bool {
    let Ok(ty) = extract_arg_type(arg) else {
        return false;
    };

    let Type::Path(type_path) = &ty else {
        return false;
    };

    let Some(segment) = type_path.path.segments.last() else {
        return false;
    };

    segment.ident == expected
}

fn extract_arg_type(arg: &FnArg) -> syn::Result<Type> {
    let FnArg::Typed(PatType { ty, .. }) = arg else {
        return Err(syn::Error::new_spanned(
            arg,
            "tool handlers must not take self",
        ));
    };

    Ok(ty.as_ref().clone())
}

fn to_pascal_case(value: &str) -> String {
    let mut output = String::new();
    let mut uppercase_next = true;

    for ch in value.chars() {
        if ch == '_' {
            uppercase_next = true;
            continue;
        }

        if uppercase_next {
            output.extend(ch.to_uppercase());
            uppercase_next = false;
        } else {
            output.push(ch);
        }
    }

    output
}
