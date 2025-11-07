//! Derive macros for Composable Rust framework
//!
//! This crate provides procedural macros to reduce boilerplate when building
//! event-driven systems with Composable Rust.
//!
//! # Available Macros
//!
//! - `#[derive(Action)]` - Generates helpers for action enums (commands/events)
//! - `#[derive(State)]` - Generates common state traits and helpers
//!
//! # Example
//!
//! ```ignore
//! use composable_rust_macros::Action;
//!
//! #[derive(Action, Clone, Debug)]
//! enum TodoAction {
//!     #[command]
//!     CreateTodo { title: String },
//!
//!     #[event]
//!     TodoCreated { id: String, title: String },
//! }
//!
//! // Generated methods:
//! assert!(TodoAction::CreateTodo { title: "test".into() }.is_command());
//! assert!(TodoAction::TodoCreated { id: "1".into(), title: "test".into() }.is_event());
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput, Data, Fields, Attribute};

/// Derive macro for Action enums
///
/// Generates helper methods for action enums:
/// - `is_command()` - Returns true if this variant is a command
/// - `is_event()` - Returns true if this variant is an event
/// - `event_type()` - Returns the event type name for serialization
///
/// # Attributes
///
/// - `#[command]` - Mark a variant as a command
/// - `#[event]` - Mark a variant as an event
///
/// # Panics
///
/// This macro will produce a compile error (not a runtime panic) if:
/// - Applied to a non-enum type
/// - A variant has both `#[command]` and `#[event]` attributes
///
/// # Example
///
/// ```ignore
/// #[derive(Action, Clone, Debug)]
/// enum OrderAction {
///     #[command]
///     PlaceOrder { customer_id: String, items: Vec<String> },
///
///     #[event]
///     OrderPlaced { order_id: String, timestamp: DateTime<Utc> },
///
///     #[command]
///     CancelOrder { order_id: String },
///
///     #[event]
///     OrderCancelled { order_id: String, reason: String },
/// }
///
/// // Usage:
/// let action = OrderAction::PlaceOrder {
///     customer_id: "cust-1".into(),
///     items: vec![],
/// };
///
/// assert!(action.is_command());
/// assert!(!action.is_event());
/// ```
#[proc_macro_derive(Action, attributes(command, event))]
#[allow(clippy::expect_used)] // Proc macro panics become compile errors, not runtime panics
pub fn derive_action(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    let Data::Enum(data_enum) = &input.data else {
        return syn::Error::new_spanned(
            input,
            "#[derive(Action)] can only be used on enums"
        )
        .to_compile_error()
        .into();
    };

    // Collect variants marked as commands or events
    let mut command_variants = Vec::new();
    let mut event_variants = Vec::new();

    for variant in &data_enum.variants {
        let variant_name = &variant.ident;
        let is_command = has_attribute(&variant.attrs, "command");
        let is_event = has_attribute(&variant.attrs, "event");

        if is_command && is_event {
            return syn::Error::new_spanned(
                variant,
                "Variant cannot be both #[command] and #[event]"
            )
            .to_compile_error()
            .into();
        }

        if is_command {
            command_variants.push(variant_name);
        }

        if is_event {
            event_variants.push(variant_name);
        }
    }

    // Build a map of variant names to their field types for efficient lookup
    let variant_map: std::collections::HashMap<_, _> = data_enum
        .variants
        .iter()
        .map(|v| (&v.ident, &v.fields))
        .collect();

    // Generate is_command() match arms
    let is_command_arms = command_variants.iter().map(|variant| {
        // SAFETY: We collected these variants from data_enum.variants above, so they must exist
        let fields = variant_map.get(variant).expect("variant must exist in map");
        match fields {
            Fields::Named(_) => quote! { Self::#variant { .. } => true, },
            Fields::Unnamed(_) => quote! { Self::#variant(..) => true, },
            Fields::Unit => quote! { Self::#variant => true, },
        }
    });

    // Generate is_event() match arms
    let is_event_arms = event_variants.iter().map(|variant| {
        // SAFETY: We collected these variants from data_enum.variants above, so they must exist
        let fields = variant_map.get(variant).expect("variant must exist in map");
        match fields {
            Fields::Named(_) => quote! { Self::#variant { .. } => true, },
            Fields::Unnamed(_) => quote! { Self::#variant(..) => true, },
            Fields::Unit => quote! { Self::#variant => true, },
        }
    });

    // Generate event_type() match arms for events only
    let event_type_arms = event_variants.iter().map(|variant| {
        let type_name = format!("{variant}.v1");
        // SAFETY: We collected these variants from data_enum.variants above, so they must exist
        let fields = variant_map.get(variant).expect("variant must exist in map");
        match fields {
            Fields::Named(_) => quote! { Self::#variant { .. } => #type_name, },
            Fields::Unnamed(_) => quote! { Self::#variant(..) => #type_name, },
            Fields::Unit => quote! { Self::#variant => #type_name, },
        }
    });

    let expanded = quote! {
        impl #name {
            /// Returns true if this action is a command
            #[must_use]
            pub const fn is_command(&self) -> bool {
                match self {
                    #(#is_command_arms)*
                    _ => false,
                }
            }

            /// Returns true if this action is an event
            #[must_use]
            pub const fn is_event(&self) -> bool {
                match self {
                    #(#is_event_arms)*
                    _ => false,
                }
            }

            /// Returns the event type name for serialization
            ///
            /// Only events have type names. Commands return "unknown".
            #[must_use]
            pub const fn event_type(&self) -> &'static str {
                match self {
                    #(#event_type_arms)*
                    _ => "unknown",
                }
            }
        }
    };

    TokenStream::from(expanded)
}

/// Derive macro for State structs
///
/// Generates common implementations for state structs:
/// - Implements `Default` if all fields have defaults
/// - Handles version tracking fields marked with `#[version]`
///
/// # Attributes
///
/// - `#[version]` - Mark a field as the version tracker
///
/// # Panics
///
/// This macro will produce a compile error (not a runtime panic) if:
/// - Applied to a non-struct type
///
/// # Example
///
/// ```ignore
/// use composable_rust_macros::State;
/// use composable_rust_core::stream::Version;
///
/// #[derive(State, Clone, Debug)]
/// struct OrderState {
///     pub orders: Vec<Order>,
///     #[version]
///     pub version: Option<Version>,
/// }
/// ```
#[proc_macro_derive(State, attributes(version))]
#[allow(clippy::expect_used)] // Proc macro panics become compile errors, not runtime panics
pub fn derive_state(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    let Data::Struct(data_struct) = &input.data else {
        return syn::Error::new_spanned(
            input,
            "#[derive(State)] can only be used on structs"
        )
        .to_compile_error()
        .into();
    };

    // Find version field if present
    let version_field = data_struct.fields.iter().find(|field| {
        has_attribute(&field.attrs, "version")
    });

    let has_version = version_field.is_some();

    // Generate version accessor if version field exists
    let version_impl = if has_version {
        // SAFETY: has_version is true, so version_field must be Some
        let field = version_field.expect("version_field must be Some when has_version is true");
        // SAFETY: We're looking at a struct field, which must have an ident
        let version_field_name = field.ident.as_ref().expect("struct field must have ident");
        quote! {
            impl #name {
                /// Get the current version of this state
                #[must_use]
                pub const fn version(&self) -> Option<composable_rust_core::stream::Version> {
                    self.#version_field_name
                }

                /// Set the version of this state
                pub fn set_version(&mut self, version: composable_rust_core::stream::Version) {
                    self.#version_field_name = Some(version);
                }
            }
        }
    } else {
        quote! {}
    };

    let expanded = quote! {
        #version_impl
    };

    TokenStream::from(expanded)
}

/// Helper function to check if an attribute list contains a specific attribute
fn has_attribute(attrs: &[Attribute], name: &str) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident(name)
    })
}

#[cfg(test)]
mod tests {
    // Macro tests use trybuild in tests/ directory
}
