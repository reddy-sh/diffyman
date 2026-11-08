//! What one top-level diff shares, down through its child documents.

use diffyman_algo::key::KeyBuilder;
use diffyman_model::{Options, Warning};

/// What a whole top-level diff shares, down through its child documents.
pub(crate) struct Context {
    /// 0 for the top-level document, plus 1 per child level (spec §19.5).
    #[cfg(feature = "archives")]
    pub depth: u32,
    /// The comparison keys of the text options, unless they failed.
    pub key_builder: Option<KeyBuilder>,
    /// Option errors, which apply whatever the kind (spec §9).
    pub option_warnings: Vec<Warning>,
    /// Archive limits and budgets (feature `archives`).
    #[cfg(feature = "archives")]
    pub tree: crate::tree::TreeState,
}

impl Context {
    /// Compiles the options once for the whole diff.
    pub fn new(options: &Options) -> Context {
        let mut option_warnings = Vec::new();
        let key_builder = crate::key_builder(options, &mut option_warnings);
        #[cfg(feature = "archives")]
        let tree = crate::tree::TreeState::new(options, &mut option_warnings);
        Context {
            #[cfg(feature = "archives")]
            depth: 0,
            key_builder,
            option_warnings,
            #[cfg(feature = "archives")]
            tree,
        }
    }
}
