//! The native VPN menu content widget.
//!
//! All state comes from the unprivileged `mvpn` CLI (`status --json`,
//! `toggles`, `fav list`) and every action shells back out to it
//! (`toggle`, `random`, `fastest`, `fav add|remove|connect`, `lockdown`,
//! `auto-connect`, `quantum`, `obf`). No daemon/IPC — `mvpn` + the Mullvad
//! daemon are the source of truth, exactly like the bar pill + Settings page.
//!
//! The connection status follows the shell's shared VPN feed
//! ([`VPN_POLL`]: `mvpn status --json --follow`) while the menu is open.
//! Favourites and toggles only change through this menu's own actions,
//! so they load on open and after each action, plus a slow safety-net
//! refresh. Nothing runs before the first open or while hidden — see
//! [`VpnMenuWidgetInput::ParentRevealChanged`].

use super::super::dns::dns_menu_widget::{
    DnsMenuWidgetInit, DnsMenuWidgetInput, DnsMenuWidgetModel,
};
use super::super::dns::state::{Mode, probe_dns_state};
use super::account::{self, AccountAction, AccountState};
use super::locations::{self, LocationAction, LocationList};
use super::multihop::{self, Entry};
use super::problem::ProblemBanner;
use super::throughput::{VpnThroughputInput, VpnThroughputModel};
use crate::menus::reveal_poll::RevealGate;
use mshell_utils::mvpn::health::{self, Fix, Problem};
use relm4::gtk::prelude::{BoxExt, ButtonExt, EditableExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller, gtk};
use std::time::Duration;

/// Safety-net full reload while open (favourites / toggles changed from
/// outside, e.g. the CLI). The status itself comes from the live feed.
const REFRESH_INTERVAL: Duration = Duration::from_secs(60);

/// How long a first click on Log out / Revoke stays armed.
const ACCOUNT_CONFIRM_WINDOW: Duration = Duration::from_secs(5);

/// Built-in height cap of the location list's scroller (used when the
/// menu has no configured height, see `list_fit`).
const LOCATIONS_LIST_CAP: i32 = 360;

/// Anti-censorship (obfuscation) modes offered in the dropdown (index ↔ string).
const OBF_MODES: &[&str] = &["auto", "off", "udp2tcp", "shadowsocks", "quic"];

use mshell_utils::mvpn::{
    VPN_POLL, VpnFav as Fav, VpnProbe, VpnStatus, locations::Country, parse_fav_list, toggle_value,
};
use mshell_utils::shared_poll::PollSubscription;

pub(crate) struct VpnMenuWidgetModel {
    connected: bool,
    /// Currently-connected relay id (empty when down).
    connected_relay: String,
    /// The status line the hero shows (`VpnStatus::summary`).
    status_text: String,
    /// The shared live VPN status; active while the menu is open.
    vpn_poll: PollSubscription<VpnProbe>,
    lockdown: bool,
    autoconnect: bool,
    quantum: bool,
    obf: String,
    favs: Vec<Fav>,
    /// Widget refs synced imperatively (avoids `#[watch] set_model` churn on
    /// the dropdown, and lets the favourites list rebuild in place).
    /// Account section: its expander (opened by the header's Account /
    /// Log in fixes), the rows box, and what it shows.
    account_expander: gtk::Expander,
    account_box: gtk::Box,
    account: AccountState,
    /// Bumped on every arm, so an old disarm timer leaves a newer arm alone.
    account_arm: u64,
    /// Error header above the hero: the problem `mvpn health` finds, with
    /// its fixes; hidden while all is well.
    problem: ProblemBanner,
    /// Hero status card — the whole card gains `.connected` (accent tint) when
    /// the tunnel is up. `state_label` = CONNECTED / DISCONNECTED,
    /// `loc_label` = the human location, `relay_label` = relay · protocol
    /// (hidden when down), `toggle_btn` swaps Connect ⇄ Disconnect.
    hero: gtk::Box,
    state_label: gtk::Label,
    loc_label: gtk::Label,
    relay_label: gtk::Label,
    toggle_btn: gtk::Button,
    /// Mode selector buttons (Mullvad / Blocky / Default) — the active one
    /// carries `.selected`. Driven from `mode`.
    mode_mullvad: gtk::Button,
    mode_blocky: gtk::Button,
    mode_default: gtk::Button,
    mode: Mode,
    lockdown_sw: gtk::Switch,
    autoconnect_sw: gtk::Switch,
    quantum_sw: gtk::Switch,
    obf_drop: gtk::DropDown,
    multihop: bool,
    entry: Entry,
    multihop_sw: gtk::Switch,
    /// Entry location pickers (country, then city) + their row, which is
    /// only sensitive while multihop is on.
    entry_row: gtk::Box,
    entry_country_drop: gtk::DropDown,
    entry_city_drop: gtk::DropDown,
    /// Set while `sync_entry` moves the pickers, so their notify handlers
    /// don't echo the change back into an `mvpn entry` call.
    entry_syncing: std::rc::Rc<std::cell::Cell<bool>>,
    fav_box: gtk::Box,
    expiry_label: gtk::Label,
    /// Location picker (lazy): the list box + a one-shot load guard.
    countries_box: gtk::Box,
    countries_loaded: bool,
    /// `mvpn locations` answered (an empty answer then reads "none").
    countries_ready: bool,
    /// Country → city tree (filtered into `countries_box` by `country_filter`).
    countries: Vec<Country>,
    /// Countries whose cities are shown.
    expanded_countries: std::collections::HashSet<String>,
    /// Live search terms for the Locations / Favourites lists.
    country_filter: String,
    fav_filter: String,
    /// Lazy-poll gates — see `ParentRevealChanged`.
    poll_started: bool,
    visible: RevealGate,
    /// Embedded DNS section (Blocky / Default / presets), collapsed by
    /// default. Its own probe loop only runs while the section is expanded
    /// AND the menu is visible — see `forward_dns_reveal`.
    dns: Controller<DnsMenuWidgetModel>,
    dns_expanded: bool,
    menu_visible: bool,
    /// Live rx/tx graph for the tunnel interface (own lazy 2 s sampler).
    throughput: Controller<VpnThroughputModel>,
}

impl std::fmt::Debug for VpnMenuWidgetModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VpnMenuWidgetModel")
            .field("connected", &self.connected)
            .finish()
    }
}

#[derive(Debug)]
pub(crate) enum VpnMenuWidgetInput {
    /// Pick a network mode — forwards `mullvad`/`blocky`/`default` to the
    /// embedded DNS widget's privileged `RunAction`.
    SelectMode(String),
    /// Bring the tunnel up/down (`mvpn toggle`) — wired to the hero button.
    Toggle,
    Random,
    /// Connect to the country the user is in (`mvpn my-country`).
    MyCountry,
    Fastest,
    AddCurrent,
    Connect(String),
    Remove(String),
    SetLockdown(bool),
    SetAutoconnect(bool),
    SetQuantum(bool),
    SetObf(u32),
    SetMultihop(bool),
    /// The entry country picker moved (index into `multihop::country_labels`).
    EntryCountry(u32),
    /// The entry city picker moved (index into `multihop::city_labels`).
    EntryCity(u32),
    /// The Settings section's expander toggled (loads the location tree
    /// the entry pickers list).
    SettingsExpanded(bool),
    RefreshNow,
    /// A fix button in the error header.
    Fix(Fix),
    /// Sent by the host menu on show/hide. The poll loop starts lazily on the
    /// first reveal and probes only while visible.
    ParentRevealChanged(bool),
    /// The DNS section's expander toggled.
    DnsExpanded(bool),
    /// The Locations section's expander toggled (loads the tree on first open).
    CountriesExpanded(bool),
    /// The Account section's expander toggled (loads it on open).
    AccountExpanded(bool),
    /// A click in the Account section.
    Account(AccountAction),
    /// Drop an armed Log out / Revoke that wasn't confirmed in time.
    AccountDisarm(u64),
    /// A click in the location list: open a country, or connect to a
    /// country / city.
    Location(LocationAction),
    /// Filter the location list (search entry in the Locations section).
    CountryFilter(String),
    /// Filter the favourites list (search entry in the Favourites section).
    FavFilter(String),
}

#[derive(Debug)]
pub(crate) enum VpnMenuWidgetOutput {}

pub(crate) struct VpnMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum VpnMenuWidgetCommandOutput {
    /// A full reload (`load`).
    Loaded(Box<Snapshot>),
    /// Mullvad's country → city tree — loaded lazily when the Locations
    /// section is first expanded.
    CountriesLoaded(Vec<Country>),
    /// `mvpn account show` / `devices` answered.
    AccountLoaded {
        account: mshell_utils::mvpn::health::Account,
        devices: Result<Vec<mshell_utils::mvpn::health::Device>, String>,
    },
    /// An account action finished: (worked, the sentence to show).
    AccountDone(bool, String),
    /// What `mvpn health` diagnosed (`None` = all well).
    Health(Option<Problem>),
    /// A connection change from the shared VPN feed (no `mvpn` spawn).
    Status {
        status: String,
        connected: bool,
        relay: String,
    },
}

/// Everything a full reload reads.
#[derive(Debug)]
pub(crate) struct Snapshot {
    status: String,
    connected: bool,
    /// Currently-connected relay id (empty when down) — used to mark the
    /// matching favourites row as active.
    relay: String,
    favs: Vec<Fav>,
    lockdown: bool,
    autoconnect: bool,
    quantum: bool,
    obf: String,
    /// Account expiry date (`YYYY-MM-DD`), from `mvpn toggles`.
    expiry: String,
    /// Current network mode (VPN / Blocky / Default) from `probe_dns_state`.
    mode: Mode,
    multihop: bool,
    /// The multihop entry (`entry=` of `mvpn toggles`).
    entry: Entry,
}

#[relm4::component(pub(crate))]
impl Component for VpnMenuWidgetModel {
    type CommandOutput = VpnMenuWidgetCommandOutput;
    type Input = VpnMenuWidgetInput;
    type Output = VpnMenuWidgetOutput;
    type Init = VpnMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "vpn-menu-widget",
            set_hexpand: false,
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 10,

            // ── Error header (hidden while all is well) ─────────
            #[local_ref]
            problem_widget -> gtk::Box {},

            // ── Hero status card ────────────────────────────────
            // The whole card gains `.connected` (accent tint) when the tunnel
            // is up. State label + dot + location + relay·protocol, then a
            // full-width Connect/Disconnect button.
            #[local_ref]
            hero_widget -> gtk::Box {
                add_css_class: "vpn-hero",
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 12,

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,

                    gtk::Image {
                        add_css_class: "vpn-hero-icon",
                        set_icon_name: Some("network-vpn-symbolic"),
                        set_valign: gtk::Align::Center,
                    },

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        set_valign: gtk::Align::Center,
                        set_spacing: 2,

                        gtk::Box {
                            set_orientation: gtk::Orientation::Horizontal,
                            set_spacing: 8,
                            set_valign: gtk::Align::Center,

                            gtk::Box {
                                add_css_class: "vpn-status-dot",
                                set_valign: gtk::Align::Center,
                            },
                            #[local_ref]
                            state_label_widget -> gtk::Label {
                                add_css_class: "vpn-state-label",
                                set_xalign: 0.0,
                            },
                        },
                        #[local_ref]
                        loc_label_widget -> gtk::Label {
                            add_css_class: "vpn-hero-location",
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_wrap_mode: gtk::pango::WrapMode::WordChar,
                        },
                        #[local_ref]
                        relay_label_widget -> gtk::Label {
                            add_css_class: "vpn-hero-relay",
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_wrap_mode: gtk::pango::WrapMode::WordChar,
                        },
                    },
                },

                #[local_ref]
                toggle_btn_widget -> gtk::Button {
                    set_css_classes: &["ok-button-primary", "vpn-toggle-btn"],
                    set_hexpand: true,
                    connect_clicked[sender] => move |_| sender.input(VpnMenuWidgetInput::Toggle),
                },
            },

            // ── Tunnel traffic (live rx/tx graph) ───────────────
            #[local_ref]
            throughput_widget -> gtk::Box {},

            // ── Network mode (Mullvad / Blocky / Default) ───────
            gtk::Label {
                add_css_class: "vpn-section-label",
                set_label: "Network",
                set_xalign: 0.0,
            },
            // Segmented: the active mode is filled with the accent. Clicks
            // forward to the embedded DnsMenuWidget's privileged `RunAction`.
            gtk::Box {
                add_css_class: "vpn-mode-row",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 6,
                set_homogeneous: true,

                #[local_ref]
                mode_mullvad_widget -> gtk::Button {
                    set_css_classes: &["ok-button-surface", "dns-action"],
                    set_label: "Mullvad",
                    connect_clicked[sender] => move |_| {
                        sender.input(VpnMenuWidgetInput::SelectMode("mullvad".into()));
                    },
                },
                #[local_ref]
                mode_blocky_widget -> gtk::Button {
                    set_css_classes: &["ok-button-surface", "dns-action"],
                    set_label: "Blocky",
                    connect_clicked[sender] => move |_| {
                        sender.input(VpnMenuWidgetInput::SelectMode("blocky".into()));
                    },
                },
                #[local_ref]
                mode_default_widget -> gtk::Button {
                    set_css_classes: &["ok-button-surface", "dns-action"],
                    set_label: "Default",
                    connect_clicked[sender] => move |_| {
                        sender.input(VpnMenuWidgetInput::SelectMode("default".into()));
                    },
                },
            },

            // ── Quick connect ───────────────────────────────────
            gtk::Label {
                add_css_class: "vpn-section-label",
                set_label: "Quick connect",
                set_xalign: 0.0,
            },
            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 6,
                set_homogeneous: true,

                gtk::Button {
                    set_css_classes: &["ok-button-surface", "dns-action"],
                    set_label: "Fastest",
                    connect_clicked[sender] => move |_| sender.input(VpnMenuWidgetInput::Fastest),
                },
                gtk::Button {
                    set_css_classes: &["ok-button-surface", "dns-action"],
                    set_label: "Random",
                    connect_clicked[sender] => move |_| sender.input(VpnMenuWidgetInput::Random),
                },
                gtk::Button {
                    set_css_classes: &["ok-button-surface", "dns-action"],
                    set_label: "My country",
                    set_tooltip_text: Some("The country you are in, or the nearest one with servers"),
                    connect_clicked[sender] => move |_| sender.input(VpnMenuWidgetInput::MyCountry),
                },
                gtk::Button {
                    set_css_classes: &["ok-button-surface", "dns-action"],
                    set_label: "Add",
                    set_tooltip_text: Some("Add the current relay to favourites"),
                    connect_clicked[sender] => move |_| sender.input(VpnMenuWidgetInput::AddCurrent),
                },
            },

            // ── Collapsible sections (all collapsed by default) ──
            // Grouped in one card with hairline separators — no bare
            // gtk::Separator stack (DESIGN.md §5 boxed-list preference).
            gtk::Box {
                add_css_class: "vpn-sections",
                set_orientation: gtk::Orientation::Vertical,

                // Favourites — search + list, lazy filtered in place.
                #[name = "fav_expander"]
                gtk::Expander {
                    add_css_class: "vpn-dns-expander",
                    set_expanded: false,

                    #[wrap(Some)]
                    set_label_widget = &gtk::Box {
                        add_css_class: "vpn-section-head",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 10,
                        gtk::Image {
                            add_css_class: "vpn-section-icon",
                            set_icon_name: Some("starred-symbolic"),
                        },
                        #[name = "fav_head_label"]
                        gtk::Label {
                            add_css_class: "vpn-section-name",
                            set_label: "Favourites",
                        },
                    },

                    #[wrap(Some)]
                    set_child = &gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_margin_top: 6,
                        set_spacing: 6,

                        gtk::SearchEntry {
                            set_placeholder_text: Some("Search favourites…"),
                            connect_search_changed[sender] => move |e| {
                                sender.input(VpnMenuWidgetInput::FavFilter(e.text().to_string()));
                            },
                        },

                        #[local_ref]
                        fav_box_widget -> gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_spacing: 4,
                        },
                    },
                },

                // Locations — lazy: the country → city tree from `mvpn locations`
                // on first expand.
                #[name = "countries_expander"]
                gtk::Expander {
                    add_css_class: "vpn-dns-expander",
                    set_expanded: false,
                    connect_expanded_notify[sender] => move |e| {
                        sender.input(VpnMenuWidgetInput::CountriesExpanded(e.is_expanded()));
                    },
                    #[wrap(Some)]
                    set_label_widget = &gtk::Box {
                        add_css_class: "vpn-section-head",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 10,
                        gtk::Image {
                            add_css_class: "vpn-section-icon",
                            set_icon_name: Some("mark-location-symbolic"),
                        },
                        gtk::Label {
                            add_css_class: "vpn-section-name",
                            set_label: "Locations",
                        },
                    },
                },

                // Account — device, paid-until, other devices, voucher, log
                // in / out. Built imperatively (`account.rs`); lazy on open.
                #[local_ref]
                account_expander_widget -> gtk::Expander {
                    add_css_class: "vpn-dns-expander",
                    set_expanded: false,
                    connect_expanded_notify[sender] => move |e| {
                        sender.input(VpnMenuWidgetInput::AccountExpanded(e.is_expanded()));
                    },
                    #[wrap(Some)]
                    set_label_widget = &gtk::Box {
                        add_css_class: "vpn-section-head",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 10,
                        gtk::Image {
                            add_css_class: "vpn-section-icon",
                            set_icon_name: Some("avatar-default-symbolic"),
                        },
                        gtk::Label {
                            add_css_class: "vpn-section-name",
                            set_label: "Account",
                        },
                    },
                },

                // DNS section (Blocky / Default / presets) — the embedded
                // DnsMenuWidget's child is attached + its expand signal wired
                // in `init`.
                #[name = "dns_expander"]
                gtk::Expander {
                    add_css_class: "vpn-dns-expander",
                    set_expanded: false,
                    connect_expanded_notify[sender] => move |e| {
                        sender.input(VpnMenuWidgetInput::DnsExpanded(e.is_expanded()));
                    },
                    #[wrap(Some)]
                    set_label_widget = &gtk::Box {
                        add_css_class: "vpn-section-head",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 10,
                        gtk::Image {
                            add_css_class: "vpn-section-icon",
                            set_icon_name: Some("security-high-symbolic"),
                        },
                        gtk::Label {
                            add_css_class: "vpn-section-name",
                            set_label: "DNS",
                        },
                    },
                },

                // Settings — lockdown / auto-connect / quantum / multihop
                // switches + anti-censorship and entry pickers, tucked away so
                // the menu is calm by default. Rows built imperatively in `init`.
                gtk::Expander {
                    add_css_class: "vpn-dns-expander",
                    set_expanded: false,
                    connect_expanded_notify[sender] => move |e| {
                        sender.input(VpnMenuWidgetInput::SettingsExpanded(e.is_expanded()));
                    },

                    #[wrap(Some)]
                    set_label_widget = &gtk::Box {
                        add_css_class: "vpn-section-head",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 10,
                        gtk::Image {
                            add_css_class: "vpn-section-icon",
                            set_icon_name: Some("emblem-system-symbolic"),
                        },
                        gtk::Label {
                            add_css_class: "vpn-section-name",
                            set_label: "Settings",
                        },
                    },

                    #[wrap(Some)]
                    set_child = &gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_margin_top: 6,

                        #[local_ref]
                        settings_box -> gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_spacing: 6,
                        },
                    },
                },
            },

            // ── Footer ──────────────────────────────────────────
            // Account expiry (dim) + Refresh, on the panel surface below the
            // sections (DESIGN.md §5 "footer is its own region").
            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 6,
                set_margin_top: 4,

                #[local_ref]
                expiry_label_widget -> gtk::Label {
                    add_css_class: "label-small",
                    add_css_class: "dim-label",
                    set_xalign: 0.0,
                    set_hexpand: true,
                    set_valign: gtk::Align::Center,
                },
                gtk::Button {
                    set_css_classes: &["ok-button-surface", "dns-action"],
                    set_label: "Refresh",
                    connect_clicked[sender] => move |_| sender.input(VpnMenuWidgetInput::RefreshNow),
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let account_expander_widget = gtk::Expander::new(None);
        let account_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        account_box.set_margin_top(6);
        account_expander_widget.set_child(Some(&account_box));
        let problem = ProblemBanner::new();
        let problem_widget = problem.widget().clone();
        // Hero status card refs — driven imperatively from `sync_view`.
        let hero_widget = gtk::Box::default();
        let state_label_widget = gtk::Label::new(Some("DISCONNECTED"));
        let loc_label_widget = gtk::Label::new(Some("Loading…"));
        let relay_label_widget = gtk::Label::new(None);
        relay_label_widget.set_visible(false);
        let toggle_btn_widget = gtk::Button::with_label("Connect");
        // Mode selector buttons — built here so they're local-ref'd into the
        // segmented row; clicks are wired in the view! macro.
        let mode_mullvad_widget = gtk::Button::new();
        let mode_blocky_widget = gtk::Button::new();
        let mode_default_widget = gtk::Button::new();

        // Build the three toggle rows + the anti-censorship dropdown, holding
        // refs so `sync_view` can drive them without `#[watch]`.
        let settings_box = gtk::Box::new(gtk::Orientation::Vertical, 6);

        let lockdown_sw = gtk::Switch::new();
        lockdown_sw.set_valign(gtk::Align::Center);
        settings_box.append(&toggle_row(
            "Lockdown mode",
            "Block all traffic when the VPN drops.",
            &lockdown_sw,
        ));
        let autoconnect_sw = gtk::Switch::new();
        autoconnect_sw.set_valign(gtk::Align::Center);
        settings_box.append(&toggle_row(
            "Auto-connect",
            "Bring the tunnel up when the daemon starts.",
            &autoconnect_sw,
        ));
        let quantum_sw = gtk::Switch::new();
        quantum_sw.set_valign(gtk::Align::Center);
        settings_box.append(&toggle_row(
            "Quantum-resistant",
            "WireGuard post-quantum key exchange.",
            &quantum_sw,
        ));

        let obf_drop =
            gtk::DropDown::new(Some(gtk::StringList::new(OBF_MODES)), gtk::Expression::NONE);
        obf_drop.set_valign(gtk::Align::Center);
        settings_box.append(&toggle_row(
            "Anti-censorship",
            "Obfuscation: auto / off / udp2tcp / shadowsocks / quic.",
            &obf_drop,
        ));

        let multihop_sw = gtk::Switch::new();
        multihop_sw.set_valign(gtk::Align::Center);
        settings_box.append(&toggle_row(
            "Multihop",
            "Enter Mullvad through a server in another location first.",
            &multihop_sw,
        ));
        let entry_country_drop = gtk::DropDown::new(
            Some(gtk::StringList::new(&["Any country"])),
            gtk::Expression::NONE,
        );
        let entry_city_drop = gtk::DropDown::new(
            Some(gtk::StringList::new(&["Any city"])),
            gtk::Expression::NONE,
        );
        entry_country_drop.set_hexpand(true);
        entry_city_drop.set_hexpand(true);
        mshell_common::a11y::name(&entry_country_drop, "Multihop entry country");
        mshell_common::a11y::name(&entry_city_drop, "Multihop entry city");
        let entry_row = entry_picker_row(&entry_country_drop, &entry_city_drop);
        entry_row.set_sensitive(false);
        settings_box.append(&entry_row);
        let entry_syncing = std::rc::Rc::new(std::cell::Cell::new(false));
        {
            let s = sender.clone();
            multihop_sw.connect_state_set(move |_, on| {
                s.input(VpnMenuWidgetInput::SetMultihop(on));
                gtk::glib::Propagation::Proceed
            });
        }
        {
            let (s, syncing) = (sender.clone(), entry_syncing.clone());
            entry_country_drop.connect_selected_notify(move |d| {
                if !syncing.get() {
                    s.input(VpnMenuWidgetInput::EntryCountry(d.selected()));
                }
            });
        }
        {
            let (s, syncing) = (sender.clone(), entry_syncing.clone());
            entry_city_drop.connect_selected_notify(move |d| {
                if !syncing.get() {
                    s.input(VpnMenuWidgetInput::EntryCity(d.selected()));
                }
            });
        }

        // Wire the toggle signals. Compare-guards in `update` stop the
        // imperative `sync_view` set from echoing back into an `mvpn` call.
        {
            let s = sender.clone();
            lockdown_sw.connect_state_set(move |_, on| {
                s.input(VpnMenuWidgetInput::SetLockdown(on));
                gtk::glib::Propagation::Proceed
            });
        }
        {
            let s = sender.clone();
            autoconnect_sw.connect_state_set(move |_, on| {
                s.input(VpnMenuWidgetInput::SetAutoconnect(on));
                gtk::glib::Propagation::Proceed
            });
        }
        {
            let s = sender.clone();
            quantum_sw.connect_state_set(move |_, on| {
                s.input(VpnMenuWidgetInput::SetQuantum(on));
                gtk::glib::Propagation::Proceed
            });
        }
        {
            let s = sender.clone();
            obf_drop.connect_selected_notify(move |d| {
                s.input(VpnMenuWidgetInput::SetObf(d.selected()));
            });
        }

        let fav_box_widget = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let expiry_label_widget = gtk::Label::new(None);
        let countries_box_widget = gtk::Box::new(gtk::Orientation::Vertical, 4);
        countries_box_widget.set_margin_top(6);

        // Embedded DNS section: Blocky / Default / presets, no VPN chrome.
        let throughput = VpnThroughputModel::builder().launch(()).detach();
        let throughput_widget = throughput.widget().clone();
        let dns = DnsMenuWidgetModel::builder()
            .launch(DnsMenuWidgetInit { embedded: true })
            .detach();

        let model = VpnMenuWidgetModel {
            connected: false,
            connected_relay: String::new(),
            status_text: String::new(),
            vpn_poll: VPN_POLL.subscribe(),
            lockdown: false,
            autoconnect: false,
            quantum: false,
            obf: "auto".to_string(),
            favs: Vec::new(),
            account_expander: account_expander_widget.clone(),
            account_box,
            account: AccountState::default(),
            account_arm: 0,
            problem,
            hero: hero_widget.clone(),
            state_label: state_label_widget.clone(),
            loc_label: loc_label_widget.clone(),
            relay_label: relay_label_widget.clone(),
            toggle_btn: toggle_btn_widget.clone(),
            mode_mullvad: mode_mullvad_widget.clone(),
            mode_blocky: mode_blocky_widget.clone(),
            mode_default: mode_default_widget.clone(),
            mode: Mode::Idle,
            lockdown_sw: lockdown_sw.clone(),
            autoconnect_sw: autoconnect_sw.clone(),
            quantum_sw: quantum_sw.clone(),
            obf_drop: obf_drop.clone(),
            multihop: false,
            entry: Entry::default(),
            multihop_sw: multihop_sw.clone(),
            entry_row: entry_row.clone(),
            entry_country_drop: entry_country_drop.clone(),
            entry_city_drop: entry_city_drop.clone(),
            entry_syncing,
            fav_box: fav_box_widget.clone(),
            expiry_label: expiry_label_widget.clone(),
            countries_box: countries_box_widget.clone(),
            countries_loaded: false,
            countries_ready: false,
            countries: Vec::new(),
            expanded_countries: std::collections::HashSet::new(),
            country_filter: String::new(),
            fav_filter: String::new(),
            poll_started: false,
            visible: RevealGate::new(false),
            dns,
            dns_expanded: false,
            menu_visible: false,
            throughput,
        };

        let widgets = view_output!();

        // Live connection status from the shared VPN feed (only probes
        // while the menu is open, see `ParentRevealChanged`).
        let mut vpn_rx = model.vpn_poll.receiver();
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            loop {
                let probe = vpn_rx.borrow_and_update().clone();
                if let Some(probe) = probe {
                    let (status, connected, relay) = status_parts(probe.status);
                    let _ = out.send(VpnMenuWidgetCommandOutput::Status {
                        status,
                        connected,
                        relay,
                    });
                }
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    r = vpn_rx.changed() => if r.is_err() { break },
                }
            }
        });
        // Drop the embedded DNS widget into the expander now that both exist.
        widgets.dns_expander.set_child(Some(model.dns.widget()));
        // Locations section = a search entry above the (filtered) list,
        // in its own scroller: 50 countries and their cities would
        // otherwise stretch the whole menu.
        let country_search = gtk::SearchEntry::new();
        country_search.set_placeholder_text(Some("Search countries and cities…"));
        {
            let s = sender.clone();
            country_search.connect_search_changed(move |e| {
                s.input(VpnMenuWidgetInput::CountryFilter(e.text().to_string()));
            });
        }
        let countries_scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .min_content_height(0)
            .propagate_natural_height(true)
            .child(&model.countries_box)
            .build();
        countries_scroller.add_css_class("vpn-locations-scroller");
        crate::menus::list_fit::mark_menu_list(&countries_scroller, LOCATIONS_LIST_CAP);
        let countries_container = gtk::Box::new(gtk::Orientation::Vertical, 6);
        countries_container.set_margin_top(6);
        countries_container.append(&country_search);
        countries_container.append(&countries_scroller);
        widgets
            .countries_expander
            .set_child(Some(&countries_container));
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            VpnMenuWidgetInput::SelectMode(id) => {
                // Run the privileged DNS-mode action via the embedded widget,
                // then re-poll shortly after so the selector + status refresh.
                let _ = self.dns.sender().send(DnsMenuWidgetInput::RunAction(id));
                sender.command(|out, _shutdown| async move {
                    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
                    let _ = out.send(load().await);
                });
            }
            VpnMenuWidgetInput::Toggle => act(&sender, vec!["toggle".into()]),
            VpnMenuWidgetInput::Random => act(&sender, vec!["random".into()]),
            VpnMenuWidgetInput::MyCountry => act(&sender, vec!["my-country".into()]),
            VpnMenuWidgetInput::Fastest => act(&sender, vec!["fastest".into()]),
            VpnMenuWidgetInput::AddCurrent => act(&sender, vec!["fav".into(), "add".into()]),
            // Connect to this specific favourite relay (`mvpn fav connect <relay>`).
            VpnMenuWidgetInput::Connect(relay) => {
                act(&sender, vec!["fav".into(), "connect".into(), relay]);
            }
            VpnMenuWidgetInput::Location(action) => {
                if let LocationAction::Toggle(code) = &action {
                    if !self.expanded_countries.remove(code) {
                        self.expanded_countries.insert(code.clone());
                    }
                    self.rebuild_locations(&sender);
                } else if let Some(args) = locations::connect_args(&action) {
                    act(&sender, args);
                }
            }
            VpnMenuWidgetInput::CountryFilter(text) => {
                self.country_filter = text;
                self.rebuild_locations(&sender);
            }
            VpnMenuWidgetInput::FavFilter(text) => {
                self.fav_filter = text;
                rebuild_favs(
                    &self.fav_box,
                    &self.favs,
                    &self.connected_relay,
                    &self.fav_filter,
                    &sender,
                );
            }
            VpnMenuWidgetInput::CountriesExpanded(expanded)
            | VpnMenuWidgetInput::SettingsExpanded(expanded) => {
                if expanded {
                    self.ensure_locations(&sender);
                }
            }
            VpnMenuWidgetInput::SetMultihop(on) => {
                self.entry_row.set_sensitive(on);
                if on != self.multihop {
                    self.multihop = on;
                    act(&sender, vec!["multihop".into(), bool_arg(on)]);
                }
            }
            VpnMenuWidgetInput::EntryCountry(index) => {
                // A new country starts at "any city" in it.
                self.set_city_labels(index, 0);
                act(&sender, multihop::entry_args(&self.countries, index, 0));
            }
            VpnMenuWidgetInput::EntryCity(index) => {
                let country = self.entry_country_drop.selected();
                act(
                    &sender,
                    multihop::entry_args(&self.countries, country, index),
                );
            }
            VpnMenuWidgetInput::Remove(relay) => {
                act(&sender, vec!["fav".into(), "remove".into(), relay]);
            }
            VpnMenuWidgetInput::SetLockdown(on) => {
                if on != self.lockdown {
                    act(&sender, vec!["lockdown".into(), bool_arg(on)]);
                }
            }
            VpnMenuWidgetInput::SetAutoconnect(on) => {
                if on != self.autoconnect {
                    act(&sender, vec!["auto-connect".into(), bool_arg(on)]);
                }
            }
            VpnMenuWidgetInput::SetQuantum(on) => {
                if on != self.quantum {
                    act(&sender, vec!["quantum".into()]); // quantum is a toggle
                }
            }
            VpnMenuWidgetInput::SetObf(idx) => {
                let mode = OBF_MODES.get(idx as usize).copied().unwrap_or("auto");
                if mode != self.obf {
                    act(&sender, vec!["obf".into(), mode.into()]);
                }
            }
            VpnMenuWidgetInput::RefreshNow => reload(&sender),
            VpnMenuWidgetInput::Fix(Fix::ShowAccount) => self.account_expander.set_expanded(true),
            VpnMenuWidgetInput::Fix(Fix::Login) => {
                // Try the saved number; the section shows the outcome and,
                // if that fails, the field to type one.
                self.account_expander.set_expanded(true);
                sender.input(VpnMenuWidgetInput::Account(AccountAction::Login(
                    String::new(),
                )));
            }
            VpnMenuWidgetInput::Fix(fix) => match fix.mvpn_args() {
                Some(args) => act(&sender, args.iter().map(|a| a.to_string()).collect()),
                None => open_account_page(),
            },
            VpnMenuWidgetInput::AccountExpanded(expanded) => {
                if expanded {
                    self.account.notice = None;
                    load_account(&sender);
                }
            }
            VpnMenuWidgetInput::Account(action) => self.account_action(action, &sender),
            VpnMenuWidgetInput::AccountDisarm(arm) => {
                if arm == self.account_arm
                    && (self.account.confirm_logout || self.account.confirm_revoke.is_some())
                {
                    self.account.confirm_logout = false;
                    self.account.confirm_revoke = None;
                    self.rebuild_account(&sender);
                }
            }
            VpnMenuWidgetInput::ParentRevealChanged(visible) => {
                self.visible.set(visible);
                self.vpn_poll.set_active(visible);
                if visible {
                    if !self.poll_started {
                        self.poll_started = true;
                        start_polling(&sender, self.visible.clone());
                    }
                    reload(&sender);
                }
                self.menu_visible = visible;
                self.throughput
                    .emit(VpnThroughputInput::ParentRevealChanged(visible));
                self.forward_dns_reveal();
            }
            VpnMenuWidgetInput::DnsExpanded(expanded) => {
                self.dns_expanded = expanded;
                self.forward_dns_reveal();
            }
        }
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        let snap = match message {
            VpnMenuWidgetCommandOutput::Loaded(snap) => *snap,
            VpnMenuWidgetCommandOutput::AccountLoaded { account, devices } => {
                self.account.account = Some(account);
                self.account.devices = Some(devices);
                self.rebuild_account(&sender);
                return;
            }
            VpnMenuWidgetCommandOutput::AccountDone(ok, text) => {
                self.account.busy = false;
                self.account.notice = Some((ok, text));
                self.rebuild_account(&sender);
                load_account(&sender);
                reload(&sender);
                return;
            }
            VpnMenuWidgetCommandOutput::Health(problem) => {
                let s = sender.clone();
                self.problem.show(
                    problem,
                    std::rc::Rc::new(move |f| s.input(VpnMenuWidgetInput::Fix(f))),
                );
                return;
            }
            VpnMenuWidgetCommandOutput::CountriesLoaded(list) => {
                self.countries = list;
                self.countries_ready = true;
                self.rebuild_locations(&sender);
                // The entry picker lists the same countries.
                self.entry_syncing.set(true);
                let labels = multihop::country_labels(&self.countries);
                let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
                self.entry_country_drop
                    .set_model(Some(&gtk::StringList::new(&refs)));
                self.entry_syncing.set(false);
                self.sync_entry();
                return;
            }
            VpnMenuWidgetCommandOutput::Status {
                status,
                connected,
                relay,
            } => {
                if (connected, relay.as_str(), status.as_str())
                    != (
                        self.connected,
                        self.connected_relay.as_str(),
                        &self.status_text,
                    )
                {
                    // A tunnel that changed state may have hit (or left) an
                    // error: re-run the diagnosis.
                    if status != self.status_text {
                        check_health(&sender);
                    }
                    self.apply_status(status, connected, relay);
                    rebuild_favs(
                        &self.fav_box,
                        &self.favs,
                        &self.connected_relay,
                        &self.fav_filter,
                        &sender,
                    );
                }
                return;
            }
        };
        let Snapshot {
            status,
            connected,
            relay,
            favs,
            lockdown,
            autoconnect,
            quantum,
            obf,
            expiry,
            mode,
            multihop,
            entry,
        } = snap;
        self.apply_status(status, connected, relay);
        self.lockdown = lockdown;
        self.autoconnect = autoconnect;
        self.quantum = quantum;
        self.obf = obf;
        self.favs = favs;
        self.mode = mode;
        self.multihop = multihop;
        self.entry = entry;

        // Highlight the active mode button. Mullvad/Mixed → Mullvad; Blocky →
        // Blocky; everything else → Default.
        let (mv, bl, df) = match mode {
            Mode::Mullvad | Mode::Mixed => (true, false, false),
            Mode::Blocky => (false, true, false),
            _ => (false, false, true),
        };
        for (btn, on) in [
            (&self.mode_mullvad, mv),
            (&self.mode_blocky, bl),
            (&self.mode_default, df),
        ] {
            if on {
                btn.add_css_class("selected");
            } else {
                btn.remove_css_class("selected");
            }
        }
        // Each `set_active` fires `connect_state_set` → an input, but the
        // compare-guard there drops it since the field already holds the value.
        self.lockdown_sw.set_active(lockdown);
        self.autoconnect_sw.set_active(autoconnect);
        self.quantum_sw.set_active(quantum);
        let sel = OBF_MODES.iter().position(|m| *m == self.obf).unwrap_or(0) as u32;
        if self.obf_drop.selected() != sel {
            self.obf_drop.set_selected(sel);
        }
        self.multihop_sw.set_active(multihop);
        self.sync_entry();

        // Account expiry line — empty until known / unparseable. Kept in the
        // layout (hexpand) so the footer Refresh button stays right-aligned.
        let show_expiry = !expiry.is_empty() && expiry != "—";
        let expiry_text = if show_expiry {
            format!("Account expires {expiry}")
        } else {
            String::new()
        };
        self.expiry_label.set_label(&expiry_text);

        // Favourites count in the section header (only when there are any).
        widgets.fav_head_label.set_label(&if self.favs.is_empty() {
            "Favourites".to_string()
        } else {
            format!("Favourites  ·  {}", self.favs.len())
        });

        rebuild_favs(
            &self.fav_box,
            &self.favs,
            &self.connected_relay,
            &self.fav_filter,
            &sender,
        );
    }
}

impl VpnMenuWidgetModel {
    /// Show a connection state on the hero card (status line, relay,
    /// connect button).
    fn apply_status(&mut self, status: String, connected: bool, relay: String) {
        self.connected = connected;
        self.connected_relay = relay;
        self.status_text = status;
        // ── Hero status card ────────────────────────────────────────
        self.state_label.set_label(if connected {
            "CONNECTED"
        } else {
            "DISCONNECTED"
        });
        if connected {
            self.hero.add_css_class("connected");
        } else {
            self.hero.remove_css_class("connected");
        }
        // Location line — reuse the human location the status string already
        // carries ("Connected · <relay> · <location>"); fall back to a plain
        // state word when the daemon didn't report a location.
        let loc_text = if connected {
            let loc = status_location(&self.status_text);
            if loc.is_empty() {
                "Connected".to_string()
            } else {
                loc
            }
        } else {
            "Not connected".to_string()
        };
        self.loc_label.set_label(&loc_text);
        // Relay · protocol — hidden when down. Mullvad WireGuard relays carry
        // "-wg-" in their id, so we can label the protocol when it's clear.
        if connected && !self.connected_relay.is_empty() {
            let proto = if self.connected_relay.contains("-wg-") {
                format!("{}  ·  WireGuard", self.connected_relay)
            } else {
                self.connected_relay.clone()
            };
            self.relay_label.set_label(&proto);
            self.relay_label.set_visible(true);
        } else {
            self.relay_label.set_visible(false);
        }
        self.toggle_btn
            .set_label(if connected { "Disconnect" } else { "Connect" });
    }

    /// Run (or arm) an Account-section action.
    fn account_action(&mut self, action: AccountAction, sender: &ComponentSender<Self>) {
        if action == AccountAction::OpenPage {
            open_account_page();
            return;
        }
        match account::command(&action, &self.account) {
            Some((args, stdin)) => {
                self.account.confirm_logout = false;
                self.account.confirm_revoke = None;
                self.account.busy = true;
                self.account.notice = None;
                self.rebuild_account(sender);
                sender.command(|out, _shutdown| async move {
                    let (ok, stdout, stderr) = run_mvpn(&args, stdin).await;
                    let _ = out.send(VpnMenuWidgetCommandOutput::AccountDone(
                        ok,
                        account::outcome(ok, &stdout, &stderr),
                    ));
                });
            }
            None => {
                // The first click on Log out / Revoke arms it for a few seconds.
                match action {
                    AccountAction::Logout => self.account.confirm_logout = true,
                    AccountAction::Revoke(name) => self.account.confirm_revoke = Some(name),
                    _ => return,
                }
                self.account_arm += 1;
                let arm = self.account_arm;
                self.rebuild_account(sender);
                let s = sender.clone();
                gtk::glib::timeout_add_local_once(ACCOUNT_CONFIRM_WINDOW, move || {
                    s.input(VpnMenuWidgetInput::AccountDisarm(arm));
                });
            }
        }
    }

    fn rebuild_account(&self, sender: &ComponentSender<Self>) {
        let s = sender.clone();
        account::rebuild(
            &self.account_box,
            &self.account,
            std::rc::Rc::new(move |a| s.input(VpnMenuWidgetInput::Account(a))),
        );
    }

    /// Fetch the location tree once (Locations list + entry pickers).
    fn ensure_locations(&mut self, sender: &ComponentSender<Self>) {
        if self.countries_loaded {
            return;
        }
        self.countries_loaded = true;
        self.rebuild_locations(sender);
        sender.command(|out, _shutdown| async move {
            let list = mshell_utils::mvpn::locations::parse(&capture(&["locations"]).await);
            let _ = out.send(VpnMenuWidgetCommandOutput::CountriesLoaded(list));
        });
    }

    /// Show `multihop` / `entry` on the switch and the pickers (without
    /// echoing the moves back as `mvpn entry` calls).
    fn sync_entry(&self) {
        self.entry_row.set_sensitive(self.multihop);
        let (country, city) = multihop::selection(&self.countries, &self.entry);
        self.entry_syncing.set(true);
        self.entry_country_drop.set_selected(country);
        self.entry_syncing.set(false);
        self.set_city_labels(country, city);
    }

    /// Refill the city picker for the country at `country` and select `city`.
    fn set_city_labels(&self, country: u32, city: u32) {
        self.entry_syncing.set(true);
        let labels = multihop::city_labels(&self.countries, country);
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        self.entry_city_drop
            .set_model(Some(&gtk::StringList::new(&refs)));
        self.entry_city_drop.set_selected(city);
        self.entry_city_drop.set_sensitive(labels.len() > 1);
        self.entry_syncing.set(false);
    }

    /// Refill the location list from the tree, search and open countries.
    fn rebuild_locations(&self, sender: &ComponentSender<Self>) {
        let s = sender.clone();
        locations::rebuild(
            &self.countries_box,
            &LocationList {
                countries: &self.countries,
                filter: &self.country_filter,
                expanded: &self.expanded_countries,
                loaded: self.countries_ready,
            },
            std::rc::Rc::new(move |a| s.input(VpnMenuWidgetInput::Location(a))),
        );
    }

    /// Drive the embedded DNS section's lazy probe: it runs only while the
    /// menu is visible *and* the DNS section is expanded.
    fn forward_dns_reveal(&self) {
        let reveal = self.menu_visible && self.dns_expanded;
        let _ = self
            .dns
            .sender()
            .send(DnsMenuWidgetInput::ParentRevealChanged(reveal));
    }
}

/// Build a DESIGN.md settings row: title + description on the left, the
/// control widget right-aligned.
fn toggle_row(title: &str, desc: &str, control: &impl gtk::prelude::IsA<gtk::Widget>) -> gtk::Box {
    use relm4::gtk::prelude::Cast;
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.add_css_class("ok-button-surface");
    let text = gtk::Box::new(gtk::Orientation::Vertical, 1);
    text.set_hexpand(true);
    let t = gtk::Label::new(Some(title));
    t.set_xalign(0.0);
    let d = gtk::Label::new(Some(desc));
    d.add_css_class("label-small");
    d.add_css_class("dim-label");
    d.set_xalign(0.0);
    d.set_wrap(true);
    d.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    text.append(&t);
    text.append(&d);
    row.append(&text);
    let control: &gtk::Widget = control.upcast_ref();
    row.append(control);
    row
}

/// The multihop entry row: title + description, then the country and city
/// pickers side by side under it (two dropdowns don't fit beside the text).
fn entry_picker_row(country: &gtk::DropDown, city: &gtk::DropDown) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 6);
    row.add_css_class("ok-button-surface");
    let t = gtk::Label::new(Some("Entry location"));
    t.set_xalign(0.0);
    let d = gtk::Label::new(Some("Where multihop enters Mullvad."));
    d.add_css_class("label-small");
    d.add_css_class("dim-label");
    d.set_xalign(0.0);
    let pickers = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    pickers.set_homogeneous(true);
    pickers.append(country);
    pickers.append(city);
    row.append(&t);
    row.append(&d);
    row.append(&pickers);
    row
}

fn bool_arg(on: bool) -> String {
    if on { "on" } else { "off" }.to_string()
}

/// Pull the human location out of the connected status string
/// (`"Connected · <relay> · <location>"`) for the hero location line —
/// everything after the second `·`, empty when the daemon reported none.
fn status_location(status: &str) -> String {
    status
        .splitn(3, " · ")
        .nth(2)
        .unwrap_or("")
        .trim()
        .to_string()
}

/// Clear + repopulate the favourites list (relay + ping, per-row connect/remove).
/// The row whose relay is `connected_relay` is marked active: its button reads
/// "Connected" and the row + button carry the `.active` accent class.
fn rebuild_favs(
    b: &gtk::Box,
    favs: &[Fav],
    connected_relay: &str,
    filter: &str,
    sender: &ComponentSender<VpnMenuWidgetModel>,
) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
    let needle = filter.trim().to_lowercase();
    let matches: Vec<&Fav> = favs
        .iter()
        .filter(|f| needle.is_empty() || f.relay.to_lowercase().contains(&needle))
        .collect();
    if matches.is_empty() {
        let msg = if favs.is_empty() {
            "No favourites yet — connect, then “Add”."
        } else {
            "No matches."
        };
        let l = gtk::Label::new(Some(msg));
        l.add_css_class("label-small");
        l.set_xalign(0.0);
        b.append(&l);
        return;
    }
    for f in matches {
        let is_active = !connected_relay.is_empty() && f.relay == connected_relay;
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.add_css_class("ok-button-surface");
        if is_active {
            row.add_css_class("active");
        }
        let name = gtk::Label::new(Some(&f.relay));
        name.set_xalign(0.0);
        name.set_hexpand(true);
        let ping = gtk::Label::new(Some(&f.ping));
        ping.add_css_class("label-small");
        let connect = gtk::Button::with_label(if is_active { "Connected" } else { "Connect" });
        connect.set_css_classes(&["ok-button-surface", "dns-action"]);
        if is_active {
            connect.add_css_class("selected");
            connect.set_sensitive(false);
        }
        let remove = gtk::Button::from_icon_name("user-trash-symbolic");
        remove.set_css_classes(&["ok-button-surface", "dns-action"]);
        mshell_common::a11y::name(&remove, "Remove relay");
        {
            let (s, r) = (sender.clone(), f.relay.clone());
            connect.connect_clicked(move |_| s.input(VpnMenuWidgetInput::Connect(r.clone())));
        }
        {
            let (s, r) = (sender.clone(), f.relay.clone());
            remove.connect_clicked(move |_| s.input(VpnMenuWidgetInput::Remove(r.clone())));
        }
        row.append(&name);
        row.append(&ping);
        row.append(&connect);
        row.append(&remove);
        b.append(&row);
    }
}

/// Spawn an `mvpn` action off-thread, then reload the menu.
fn act(sender: &ComponentSender<VpnMenuWidgetModel>, args: Vec<String>) {
    sender.command(|out, _shutdown| async move {
        let _ = tokio::process::Command::new("mvpn")
            .args(&args)
            .status()
            .await;
        let _ = out.send(load().await);
        let _ = out.send(VpnMenuWidgetCommandOutput::Health(load_health().await));
    });
}

fn reload(sender: &ComponentSender<VpnMenuWidgetModel>) {
    sender.command(|out, _shutdown| async move {
        let _ = out.send(load().await);
        let _ = out.send(VpnMenuWidgetCommandOutput::Health(load_health().await));
    });
}

/// Read the Account section's data off-thread.
fn load_account(sender: &ComponentSender<VpnMenuWidgetModel>) {
    use mshell_utils::mvpn::health::{parse_account, parse_devices};
    sender.command(|out, _shutdown| async move {
        let account = parse_account(&capture(&["account", "show", "--json"]).await);
        let devices = match parse_devices(&capture(&["account", "devices", "--json"]).await) {
            Some(list) => Ok(list),
            None => Err("The device list can't be read right now.".to_string()),
        };
        let _ = out.send(VpnMenuWidgetCommandOutput::AccountLoaded { account, devices });
    });
}

/// Run `mvpn <args>`, writing `stdin` (a secret: the account number, a
/// voucher) to its stdin; (worked, stdout, stderr).
async fn run_mvpn(args: &[String], stdin: Option<String>) -> (bool, String, String) {
    use std::process::Stdio;
    use tokio::io::AsyncWriteExt as _;
    let child = tokio::process::Command::new("mvpn")
        .args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let Ok(mut child) = child else {
        return (false, String::new(), "mvpn is not installed".into());
    };
    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        let _ = pipe.write_all(text.as_bytes()).await;
        drop(pipe);
    }
    match child.wait_with_output().await {
        Ok(o) => (
            o.status.success(),
            String::from_utf8_lossy(&o.stdout).into_owned(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        ),
        Err(_) => (false, String::new(), String::new()),
    }
}

fn open_account_page() {
    if let Err(e) = mshell_process::desktop::open(health::ACCOUNT_URL) {
        tracing::warn!("vpn: opening the account page failed: {e}");
    }
}

fn check_health(sender: &ComponentSender<VpnMenuWidgetModel>) {
    sender.command(|out, _shutdown| async move {
        let _ = out.send(VpnMenuWidgetCommandOutput::Health(load_health().await));
    });
}

/// `mvpn health --json` → the problem to show. An `mvpn` too old for
/// `health` (or none at all) shows nothing rather than a guess.
async fn load_health() -> Option<Problem> {
    health::parse(&capture(&["health", "--json"]).await).and_then(|h| health::diagnose(&h))
}

/// Poll loop, started lazily on first reveal; it waits on the `visible`
/// gate, so a hidden menu arms no timer (the reveal handler reloads).
fn start_polling(sender: &ComponentSender<VpnMenuWidgetModel>, visible: RevealGate) {
    sender.command(move |out, shutdown| async move {
        let shutdown_fut = shutdown.wait();
        tokio::pin!(shutdown_fut);
        loop {
            let resumed = tokio::select! {
                () = &mut shutdown_fut => break,
                r = visible.tick(REFRESH_INTERVAL) => r,
            };
            if !resumed {
                let _ = out.send(load().await);
                let _ = out.send(VpnMenuWidgetCommandOutput::Health(load_health().await));
            }
        }
    });
}

/// `(status line, connected, relay)` for the hero card; the relay is
/// empty while down.
fn status_parts(vpn: VpnStatus) -> (String, bool, String) {
    let status = vpn.summary();
    let relay = if vpn.connected {
        vpn.relay
    } else {
        String::new()
    };
    (status, vpn.connected, relay)
}

/// Query `mvpn` for status + toggles + favourites.
async fn load() -> VpnMenuWidgetCommandOutput {
    let (status, connected, relay) =
        status_parts(VpnStatus::parse(&capture(&["status", "--json"]).await));
    let favs = parse_fav_list(&capture(&["fav", "list"]).await);
    let toggles = capture(&["toggles"]).await;
    let kv = |key: &str| toggle_value(&toggles, key);
    // Current network mode for the top selector (VPN / Blocky / Default).
    let mode = probe_dns_state().await.mode_id();
    VpnMenuWidgetCommandOutput::Loaded(Box::new(Snapshot {
        status,
        connected,
        relay,
        favs,
        lockdown: kv("lockdown") == "on",
        autoconnect: kv("autoconnect") == "on",
        quantum: kv("quantum") == "on",
        obf: {
            let m = kv("obf");
            if m.is_empty() { "auto".to_string() } else { m }
        },
        expiry: kv("expiry"),
        mode,
        multihop: kv("multihop") == "on",
        entry: Entry::parse(&kv("entry")),
    }))
}

async fn capture(args: &[&str]) -> String {
    tokio::process::Command::new("mvpn")
        .args(args)
        .output()
        .await
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{VpnStatus, status_parts};

    /// The live feed and the full reload fill the hero card the same way.
    #[test]
    fn feed_status_fills_the_hero_like_a_reload() {
        let up = VpnStatus::parse(
            r#"{"connected":true,"relay":"se-got-wg-001","location":"Gothenburg"}"#,
        );
        assert_eq!(
            status_parts(up),
            (
                "Connected · se-got-wg-001 · Gothenburg".to_string(),
                true,
                "se-got-wg-001".to_string()
            )
        );
        let down = VpnStatus::parse(r#"{"connected":false,"relay":"se-got-wg-001"}"#);
        assert_eq!(
            status_parts(down),
            ("Disconnected".to_string(), false, String::new())
        );
    }
}
