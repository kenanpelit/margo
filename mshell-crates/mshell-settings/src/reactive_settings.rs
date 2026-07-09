//! `reactive_settings!` — the two-way binding between a Settings page and the
//! shared `config_manager` reactive store, generated once instead of hand-rolled
//! per field on every page.
//!
//! ## What each scalar setting needs, and what this generates
//!
//! A bound field carries the same five-part shape on every page: a `<Base>Changed`
//! input (widget → config) and a `<Base>Effect` input (external config change →
//! model), a scoped effect that re-pushes `<Base>Effect` whenever the store value
//! changes, a `get_untracked()` snapshot for the initial model, and the two match
//! arms that service those inputs. This macro emits all of it: the `pub(crate) enum
//! $input` (both variants per field), plus an inherent `impl $model` with
//! `from_config_store` (effects + initial snapshot) and `apply_reactive` (the arms).
//!
//! ## Field name vs config key are independent, so both are required
//!
//! The model field, the enum variant base, and the config key are three separate
//! identifiers that only *often* coincide. `idle` binds model field `dim_timeout`
//! to config key `dim_timeout_minutes`; `game_mode` names the variant base
//! `Animations` over model/config field `disable_animations`. Deriving any one from
//! another would silently miswire those pages, so each field entry spells out all
//! three: `Base => field: Type => config_key`.
//!
//! ## Constraints on a page that adopts this
//!
//! - The variant bases must match the `<Base>Changed` names already spelled in the
//!   page's untouched `view!` block — this macro reproduces them verbatim, it does
//!   not rename anything.
//! - The model must be exactly the bound fields plus `_effects: EffectScope`; the
//!   generated `from_config_store` returns the whole `Self`, so an extra field is a
//!   compile error (the signal to leave that page hand-rolled).
//! - The enum is generated whole, so a page with extra input variants (buttons,
//!   list actions) cannot use this — a `macro_rules!` cannot splice variants into a
//!   hand-written enum.
//! - Resolution is call-site: the adopting module must have `config_manager`,
//!   `EffectScope`, `Get`, `GetUntracked`, and its `*StoreFields` traits in scope
//!   (every current page already does), which also keeps those `use`s live.
//! - `clamped { … in lo ..= hi }` fields clamp on the config write only, matching
//!   the hand-written pages where the widget range is defensive-clamped before it
//!   reaches the store; the `Effect` (store → model) path is never clamped.

/// See the module docs for the field/key/variant distinction and the page
/// constraints. Grammar: a mandatory `fields { … }` block of unclamped bindings and
/// an optional `clamped { … in lo ..= hi }` block whose `Changed` writes clamp.
macro_rules! reactive_settings {
    (
        model: $model:ident,
        input: $input:ident,
        group: $group:ident,
        fields {
            $( $pb:ident => $pf:ident : $pt:ty => $pk:ident ),* $(,)?
        }
        $(
            clamped {
                $( $cb:ident => $cf:ident : $ct:ty => $ck:ident in $lo:literal ..= $hi:literal ),* $(,)?
            }
        )?
    ) => {
        ::paste::paste! {
            #[derive(Debug)]
            pub(crate) enum $input {
                $(
                    [<$pb Changed>]($pt),
                    [<$pb Effect>]($pt),
                )*
                $($(
                    [<$cb Changed>]($ct),
                    [<$cb Effect>]($ct),
                )*)?
            }

            impl $model {
                /// Snapshot every bound field via `get_untracked` and register a
                /// scoped effect per field that re-pushes its `Effect` input on any
                /// external store change. The returned `Self` owns the `EffectScope`,
                /// so the effects live exactly as long as the component.
                fn from_config_store(sender: &relm4::ComponentSender<Self>) -> Self {
                    let mut effects = EffectScope::new();
                    $(
                        let sc = sender.clone();
                        effects.push(move |_| {
                            let v = config_manager().config().$group().$pk().get();
                            sc.input($input::[<$pb Effect>](v));
                        });
                    )*
                    $($(
                        let sc = sender.clone();
                        effects.push(move |_| {
                            let v = config_manager().config().$group().$ck().get();
                            sc.input($input::[<$cb Effect>](v));
                        });
                    )*)?
                    Self {
                        $( $pf: config_manager().config().$group().$pk().get_untracked(), )*
                        $($( $cf: config_manager().config().$group().$ck().get_untracked(), )*)?
                        _effects: effects,
                    }
                }

                /// Service a bound input: `Changed` writes the widget value back to
                /// the store, `Effect` mirrors a store change into the model. The
                /// match is exhaustive over the generated enum, so a page adopting
                /// extra variants would fail to compile here.
                fn apply_reactive(&mut self, message: $input) {
                    match message {
                        $(
                            $input::[<$pb Changed>](v) => {
                                config_manager().update_config(move |c| c.$group.$pk = v);
                            }
                            $input::[<$pb Effect>](v) => self.$pf = v,
                        )*
                        $($(
                            $input::[<$cb Changed>](v) => {
                                config_manager().update_config(move |c| c.$group.$ck = v.clamp($lo, $hi));
                            }
                            $input::[<$cb Effect>](v) => self.$cf = v,
                        )*)?
                    }
                }
            }
        }
    };
}
