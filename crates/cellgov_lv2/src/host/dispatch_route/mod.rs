//! Top-level dispatch routing for [`super::Lv2Host`]. The match itself
//! lives in [`dispatch`]; per-arm dispatch helpers live in
//! [`inline_arms`] and [`unsupported_arms`]; shared effect builders
//! in [`helpers`].

mod dispatch;
mod helpers;
mod inline_arms;
mod unsupported_arms;

#[cfg(test)]
mod tests;
