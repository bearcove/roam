macro_rules! declare_id {
    ($(#[$meta:meta])* $name:ident, $inner:ty) => {
        $(#[$meta])*
        #[derive(Facet, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Clone, Copy)]
        #[repr(transparent)]
        #[facet(transparent)]
        pub struct $name(pub $inner);

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

mod rpc_plan;
pub use rpc_plan::*;

mod roam_error;
pub use roam_error::*;

mod services;
pub use services::*;

mod requests;
pub use requests::*;

mod message;
pub use message::*;

mod selfref;
pub use selfref::*;

mod link;
pub use link::*;

mod conduit;
pub use conduit::*;

mod metadata;
pub use metadata::*;

mod channel;
pub use channel::*;
