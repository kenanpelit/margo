//! Per-connection editor for NetworkManager connections.
//!
//! Covers General (autoconnect, metered), IPv4, IPv6 (method + manual
//! address/gateway/dns/dns-search/routes), and optionally Wi-Fi Security
//! (write-only PSK). Enterprise EAP cert UI is intentionally excluded
//! per YAGNI — a comment marks where it would slot in.
//!
//! Integration: the editor is embedded inside the Network settings page as
//! a child of its internal `gtk::Stack`; no separate toplevel is created.

use std::net::IpAddr;

use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, ButtonExt, EditableExt, EntryExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

use crate::net::nmcli;

// ── EditorFields ─────────────────────────────────────────────────────────────

/// All readable fields for a connection; populated by the background read.
#[derive(Debug, Default, Clone)]
pub(crate) struct EditorFields {
    // General
    pub autoconnect: bool,
    /// "yes" / "no" / "unknown" / "" (treat "" as "unknown")
    pub metered: String,
    // IPv4
    pub ipv4_method: String,
    pub ipv4_addresses: String,
    pub ipv4_gateway: String,
    pub ipv4_dns: String,
    pub ipv4_dns_search: String,
    pub ipv4_routes: String,
    // IPv6
    pub ipv6_method: String,
    pub ipv6_addresses: String,
    pub ipv6_gateway: String,
    pub ipv6_dns: String,
    pub ipv6_dns_search: String,
    pub ipv6_routes: String,
}

// ── Validation helpers (pure — testable) ─────────────────────────────────────

/// Return `true` if every non-empty token in `s` (split on `,` and whitespace)
/// parses as a valid IP address. An empty string is considered valid (the field
/// will just be left unset).
pub(crate) fn all_ips_valid(s: &str) -> bool {
    s.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|t| !t.is_empty())
        .all(|t| t.parse::<IpAddr>().is_ok())
}

/// Return `true` if `s` is a valid CIDR address ("ip/prefix").
pub(crate) fn cidr_valid(s: &str) -> bool {
    let Some((ip, prefix)) = s.split_once('/') else {
        return false;
    };
    let Ok(_ip) = ip.parse::<IpAddr>() else {
        return false;
    };
    prefix.parse::<u8>().is_ok()
}

/// Validate a list of CIDR addresses (space- or comma-separated).
/// Empty string is valid.
fn cidr_list_valid(s: &str) -> bool {
    s.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|t| !t.is_empty())
        .all(cidr_valid)
}

// ── IP method options ─────────────────────────────────────────────────────────

const IPV4_METHODS: &[&str] = &["auto", "manual", "link-local", "shared", "disabled"];
const IPV6_METHODS: &[&str] = &[
    "auto",
    "manual",
    "link-local",
    "shared",
    "ignore",
    "disabled",
];
const METERED_OPTIONS: &[&str] = &["unknown", "yes", "no"];

fn method_index(methods: &[&str], value: &str) -> u32 {
    methods.iter().position(|&m| m == value).unwrap_or(0) as u32
}

fn metered_index(value: &str) -> u32 {
    match value {
        "yes" => 1,
        "no" => 2,
        _ => 0, // unknown / empty
    }
}

fn metered_from_index(idx: u32) -> &'static str {
    match idx {
        1 => "yes",
        2 => "no",
        _ => "unknown",
    }
}

// ── Model ─────────────────────────────────────────────────────────────────────

pub(crate) struct ConnectionEditorModel {
    uuid: String,
    conn_name: String,
    is_wifi: bool,

    // form state
    autoconnect: bool,
    metered_idx: u32,
    ipv4_method_idx: u32,
    ipv4_addresses: String,
    ipv4_gateway: String,
    ipv4_dns: String,
    ipv4_dns_search: String,
    ipv4_routes: String,
    ipv6_method_idx: u32,
    ipv6_addresses: String,
    ipv6_gateway: String,
    ipv6_dns: String,
    ipv6_dns_search: String,
    ipv6_routes: String,
    psk: String,

    // UI state
    loading: bool,
    error: String,
}

impl Default for ConnectionEditorModel {
    fn default() -> Self {
        Self {
            uuid: String::new(),
            conn_name: String::new(),
            is_wifi: false,
            autoconnect: true,
            metered_idx: 0,
            ipv4_method_idx: 0,
            ipv4_addresses: String::new(),
            ipv4_gateway: String::new(),
            ipv4_dns: String::new(),
            ipv4_dns_search: String::new(),
            ipv4_routes: String::new(),
            ipv6_method_idx: 0,
            ipv6_addresses: String::new(),
            ipv6_gateway: String::new(),
            ipv6_dns: String::new(),
            ipv6_dns_search: String::new(),
            ipv6_routes: String::new(),
            psk: String::new(),
            loading: false,
            error: String::new(),
        }
    }
}

// ── Input / Output ────────────────────────────────────────────────────────────

#[derive(Debug)]
pub(crate) enum ConnectionEditorInput {
    /// Trigger a load for the given UUID / display name / wifi flag.
    Load(String, String, bool),
    /// Internal: fields have been read from nmcli on a background task.
    Loaded(EditorFields),
    // Field edits
    AutoconnectChanged(bool),
    MeteredChanged(u32),
    Ipv4MethodChanged(u32),
    Ipv4AddressesChanged(String),
    Ipv4GatewayChanged(String),
    Ipv4DnsChanged(String),
    Ipv4DnsSearchChanged(String),
    Ipv4RoutesChanged(String),
    Ipv6MethodChanged(u32),
    Ipv6AddressesChanged(String),
    Ipv6GatewayChanged(String),
    Ipv6DnsChanged(String),
    Ipv6DnsSearchChanged(String),
    Ipv6RoutesChanged(String),
    PskChanged(String),
    /// Apply and write back via nmcli.
    Apply,
    /// Go back without saving.
    Back,
}

#[derive(Debug)]
pub(crate) enum ConnectionEditorOutput {
    /// Emitted on Back and after a successful Apply — parent should switch
    /// back to the list view and reload connections.
    Closed,
}

// ── Component ─────────────────────────────────────────────────────────────────

// The #[watch] set_stack requires `&Stack` which clippy misidentifies as an
// unnecessary borrow because the macro wraps the expression in generated code.
#[allow(clippy::needless_borrow)]
#[relm4::component(pub)]
impl Component for ConnectionEditorModel {
    type CommandOutput = ();
    type Input = ConnectionEditorInput;
    type Output = ConnectionEditorOutput;
    type Init = ();

    view! {
        #[root]
        gtk::Box {
            add_css_class: "settings-page",
            set_orientation: gtk::Orientation::Vertical,
            set_hexpand: true,
            set_vexpand: true,
            set_spacing: 12,

            // ── Header row: Back button + connection name ─────────────
            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 8,

                gtk::Button {
                    add_css_class: "ok-button-primary",
                    set_icon_name: "go-previous-symbolic",
                    set_tooltip_text: Some("Back to network list"),
                    connect_clicked[sender] => move |_| {
                        sender.input(ConnectionEditorInput::Back);
                    },
                },

                gtk::Label {
                    add_css_class: "settings-hero-title",
                    set_hexpand: true,
                    set_halign: gtk::Align::Start,
                    set_valign: gtk::Align::Center,
                    set_ellipsize: gtk::pango::EllipsizeMode::End,
                    #[watch]
                    set_label: &model.conn_name,
                },

                gtk::Spinner {
                    #[watch]
                    set_spinning: model.loading,
                    #[watch]
                    set_visible: model.loading,
                    set_valign: gtk::Align::Center,
                },
            },

            // ── Inline error label ─────────────────────────────────────
            gtk::Label {
                add_css_class: "status-error",
                set_halign: gtk::Align::Start,
                set_xalign: 0.0,
                set_wrap: true,
                #[watch]
                set_label: &model.error,
                #[watch]
                set_visible: !model.error.is_empty(),
            },

            // ── Tab switcher ──────────────────────────────────────────
            gtk::StackSwitcher {
                set_hexpand: true,
                #[watch]
                set_stack: Some(&tab_stack),
            },

            // ── Tab stack ─────────────────────────────────────────────
            #[name = "tab_stack"]
            gtk::Stack {
                set_hexpand: true,
                set_vexpand: true,
                set_transition_type: gtk::StackTransitionType::SlideLeftRight,

                // ── General tab ───────────────────────────────────────
                add_titled[Some("general"), "General"] = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 12,
                    set_margin_top: 12,
                    set_margin_bottom: 12,
                    set_margin_start: 8,
                    set_margin_end: 8,

                    // Autoconnect row
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,

                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Autoconnect",
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Automatically connect when available.",
                                set_xalign: 0.0,
                                set_wrap: true,
                            },
                        },

                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(autoconnect_handler)]
                            set_active: model.autoconnect,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(ConnectionEditorInput::AutoconnectChanged(v));
                                glib::Propagation::Proceed
                            } @autoconnect_handler,
                        },
                    },

                    // Metered row
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,

                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Metered",
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Whether this connection counts against a data cap.",
                                set_xalign: 0.0,
                                set_wrap: true,
                            },
                        },

                        gtk::DropDown {
                            set_valign: gtk::Align::Center,
                            set_model: Some(&gtk::StringList::new(METERED_OPTIONS)),
                            #[watch]
                            #[block_signal(metered_handler)]
                            set_selected: model.metered_idx,
                            connect_selected_notify[sender] => move |dd| {
                                sender.input(ConnectionEditorInput::MeteredChanged(dd.selected()));
                            } @metered_handler,
                        },
                    },
                },

                // ── IPv4 tab ──────────────────────────────────────────
                add_titled[Some("ipv4"), "IPv4"] = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 12,
                    set_margin_top: 12,
                    set_margin_bottom: 12,
                    set_margin_start: 8,
                    set_margin_end: 8,

                    // Method row
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,

                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_hexpand: true,
                            set_halign: gtk::Align::Start,
                            set_valign: gtk::Align::Center,
                            set_label: "Method",
                        },

                        gtk::DropDown {
                            set_valign: gtk::Align::Center,
                            set_model: Some(&gtk::StringList::new(IPV4_METHODS)),
                            #[watch]
                            #[block_signal(ipv4_method_handler)]
                            set_selected: model.ipv4_method_idx,
                            connect_selected_notify[sender] => move |dd| {
                                sender.input(ConnectionEditorInput::Ipv4MethodChanged(dd.selected()));
                            } @ipv4_method_handler,
                        },
                    },

                    // Manual fields (revealed when method == "manual")
                    gtk::Revealer {
                        set_transition_type: gtk::RevealerTransitionType::SlideDown,
                        #[watch]
                        set_reveal_child: IPV4_METHODS.get(model.ipv4_method_idx as usize)
                            == Some(&"manual"),

                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_spacing: 8,
                            set_margin_top: 4,

                            gtk::Box {
                                set_orientation: gtk::Orientation::Horizontal,
                                set_spacing: 12,
                                gtk::Label {
                                    add_css_class: "label-medium-bold",
                                    set_width_chars: 12,
                                    set_halign: gtk::Align::Start,
                                    set_valign: gtk::Align::Center,
                                    set_label: "Addresses",
                                },
                                gtk::Entry {
                                    set_hexpand: true,
                                    set_placeholder_text: Some("e.g. 192.168.1.50/24"),
                                    #[watch]
                                    #[block_signal(ipv4_addr_handler)]
                                    set_text: &model.ipv4_addresses,
                                    connect_changed[sender] => move |e| {
                                        sender.input(ConnectionEditorInput::Ipv4AddressesChanged(
                                            e.text().to_string(),
                                        ));
                                    } @ipv4_addr_handler,
                                },
                            },

                            gtk::Box {
                                set_orientation: gtk::Orientation::Horizontal,
                                set_spacing: 12,
                                gtk::Label {
                                    add_css_class: "label-medium-bold",
                                    set_width_chars: 12,
                                    set_halign: gtk::Align::Start,
                                    set_valign: gtk::Align::Center,
                                    set_label: "Gateway",
                                },
                                gtk::Entry {
                                    set_hexpand: true,
                                    set_placeholder_text: Some("e.g. 192.168.1.1"),
                                    #[watch]
                                    #[block_signal(ipv4_gw_handler)]
                                    set_text: &model.ipv4_gateway,
                                    connect_changed[sender] => move |e| {
                                        sender.input(ConnectionEditorInput::Ipv4GatewayChanged(
                                            e.text().to_string(),
                                        ));
                                    } @ipv4_gw_handler,
                                },
                            },

                            gtk::Box {
                                set_orientation: gtk::Orientation::Horizontal,
                                set_spacing: 12,
                                gtk::Label {
                                    add_css_class: "label-medium-bold",
                                    set_width_chars: 12,
                                    set_halign: gtk::Align::Start,
                                    set_valign: gtk::Align::Center,
                                    set_label: "DNS",
                                },
                                gtk::Entry {
                                    set_hexpand: true,
                                    set_placeholder_text: Some("e.g. 1.1.1.1 8.8.8.8"),
                                    #[watch]
                                    #[block_signal(ipv4_dns_handler)]
                                    set_text: &model.ipv4_dns,
                                    connect_changed[sender] => move |e| {
                                        sender.input(ConnectionEditorInput::Ipv4DnsChanged(
                                            e.text().to_string(),
                                        ));
                                    } @ipv4_dns_handler,
                                },
                            },

                            gtk::Box {
                                set_orientation: gtk::Orientation::Horizontal,
                                set_spacing: 12,
                                gtk::Label {
                                    add_css_class: "label-medium-bold",
                                    set_width_chars: 12,
                                    set_halign: gtk::Align::Start,
                                    set_valign: gtk::Align::Center,
                                    set_label: "DNS Search",
                                },
                                gtk::Entry {
                                    set_hexpand: true,
                                    set_placeholder_text: Some("e.g. example.com"),
                                    #[watch]
                                    #[block_signal(ipv4_dns_search_handler)]
                                    set_text: &model.ipv4_dns_search,
                                    connect_changed[sender] => move |e| {
                                        sender.input(ConnectionEditorInput::Ipv4DnsSearchChanged(
                                            e.text().to_string(),
                                        ));
                                    } @ipv4_dns_search_handler,
                                },
                            },

                            gtk::Box {
                                set_orientation: gtk::Orientation::Horizontal,
                                set_spacing: 12,
                                gtk::Label {
                                    add_css_class: "label-medium-bold",
                                    set_width_chars: 12,
                                    set_halign: gtk::Align::Start,
                                    set_valign: gtk::Align::Center,
                                    set_label: "Routes",
                                },
                                gtk::Entry {
                                    set_hexpand: true,
                                    set_placeholder_text: Some("e.g. 10.0.0.0/8 192.168.1.1"),
                                    #[watch]
                                    #[block_signal(ipv4_routes_handler)]
                                    set_text: &model.ipv4_routes,
                                    connect_changed[sender] => move |e| {
                                        sender.input(ConnectionEditorInput::Ipv4RoutesChanged(
                                            e.text().to_string(),
                                        ));
                                    } @ipv4_routes_handler,
                                },
                            },
                        },
                    },
                },

                // ── IPv6 tab ──────────────────────────────────────────
                add_titled[Some("ipv6"), "IPv6"] = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 12,
                    set_margin_top: 12,
                    set_margin_bottom: 12,
                    set_margin_start: 8,
                    set_margin_end: 8,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,

                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_hexpand: true,
                            set_halign: gtk::Align::Start,
                            set_valign: gtk::Align::Center,
                            set_label: "Method",
                        },

                        gtk::DropDown {
                            set_valign: gtk::Align::Center,
                            set_model: Some(&gtk::StringList::new(IPV6_METHODS)),
                            #[watch]
                            #[block_signal(ipv6_method_handler)]
                            set_selected: model.ipv6_method_idx,
                            connect_selected_notify[sender] => move |dd| {
                                sender.input(ConnectionEditorInput::Ipv6MethodChanged(dd.selected()));
                            } @ipv6_method_handler,
                        },
                    },

                    gtk::Revealer {
                        set_transition_type: gtk::RevealerTransitionType::SlideDown,
                        #[watch]
                        set_reveal_child: IPV6_METHODS.get(model.ipv6_method_idx as usize)
                            == Some(&"manual"),

                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_spacing: 8,
                            set_margin_top: 4,

                            gtk::Box {
                                set_orientation: gtk::Orientation::Horizontal,
                                set_spacing: 12,
                                gtk::Label {
                                    add_css_class: "label-medium-bold",
                                    set_width_chars: 12,
                                    set_halign: gtk::Align::Start,
                                    set_valign: gtk::Align::Center,
                                    set_label: "Addresses",
                                },
                                gtk::Entry {
                                    set_hexpand: true,
                                    set_placeholder_text: Some("e.g. 2001:db8::1/64"),
                                    #[watch]
                                    #[block_signal(ipv6_addr_handler)]
                                    set_text: &model.ipv6_addresses,
                                    connect_changed[sender] => move |e| {
                                        sender.input(ConnectionEditorInput::Ipv6AddressesChanged(
                                            e.text().to_string(),
                                        ));
                                    } @ipv6_addr_handler,
                                },
                            },

                            gtk::Box {
                                set_orientation: gtk::Orientation::Horizontal,
                                set_spacing: 12,
                                gtk::Label {
                                    add_css_class: "label-medium-bold",
                                    set_width_chars: 12,
                                    set_halign: gtk::Align::Start,
                                    set_valign: gtk::Align::Center,
                                    set_label: "Gateway",
                                },
                                gtk::Entry {
                                    set_hexpand: true,
                                    set_placeholder_text: Some("e.g. 2001:db8::1"),
                                    #[watch]
                                    #[block_signal(ipv6_gw_handler)]
                                    set_text: &model.ipv6_gateway,
                                    connect_changed[sender] => move |e| {
                                        sender.input(ConnectionEditorInput::Ipv6GatewayChanged(
                                            e.text().to_string(),
                                        ));
                                    } @ipv6_gw_handler,
                                },
                            },

                            gtk::Box {
                                set_orientation: gtk::Orientation::Horizontal,
                                set_spacing: 12,
                                gtk::Label {
                                    add_css_class: "label-medium-bold",
                                    set_width_chars: 12,
                                    set_halign: gtk::Align::Start,
                                    set_valign: gtk::Align::Center,
                                    set_label: "DNS",
                                },
                                gtk::Entry {
                                    set_hexpand: true,
                                    set_placeholder_text: Some("e.g. 2606:4700:4700::1111"),
                                    #[watch]
                                    #[block_signal(ipv6_dns_handler)]
                                    set_text: &model.ipv6_dns,
                                    connect_changed[sender] => move |e| {
                                        sender.input(ConnectionEditorInput::Ipv6DnsChanged(
                                            e.text().to_string(),
                                        ));
                                    } @ipv6_dns_handler,
                                },
                            },

                            gtk::Box {
                                set_orientation: gtk::Orientation::Horizontal,
                                set_spacing: 12,
                                gtk::Label {
                                    add_css_class: "label-medium-bold",
                                    set_width_chars: 12,
                                    set_halign: gtk::Align::Start,
                                    set_valign: gtk::Align::Center,
                                    set_label: "DNS Search",
                                },
                                gtk::Entry {
                                    set_hexpand: true,
                                    set_placeholder_text: Some("e.g. example.com"),
                                    #[watch]
                                    #[block_signal(ipv6_dns_search_handler)]
                                    set_text: &model.ipv6_dns_search,
                                    connect_changed[sender] => move |e| {
                                        sender.input(ConnectionEditorInput::Ipv6DnsSearchChanged(
                                            e.text().to_string(),
                                        ));
                                    } @ipv6_dns_search_handler,
                                },
                            },

                            gtk::Box {
                                set_orientation: gtk::Orientation::Horizontal,
                                set_spacing: 12,
                                gtk::Label {
                                    add_css_class: "label-medium-bold",
                                    set_width_chars: 12,
                                    set_halign: gtk::Align::Start,
                                    set_valign: gtk::Align::Center,
                                    set_label: "Routes",
                                },
                                gtk::Entry {
                                    set_hexpand: true,
                                    set_placeholder_text: Some("e.g. ::/0 2001:db8::1"),
                                    #[watch]
                                    #[block_signal(ipv6_routes_handler)]
                                    set_text: &model.ipv6_routes,
                                    connect_changed[sender] => move |e| {
                                        sender.input(ConnectionEditorInput::Ipv6RoutesChanged(
                                            e.text().to_string(),
                                        ));
                                    } @ipv6_routes_handler,
                                },
                            },
                        },
                    },
                },

                // ── Security tab (always present; hidden for non-Wi-Fi) ───
                add_titled[Some("security"), "Security"] = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 12,
                    set_margin_top: 12,
                    set_margin_bottom: 12,
                    set_margin_start: 8,
                    set_margin_end: 8,
                    #[watch]
                    set_visible: model.is_wifi,

                    gtk::Label {
                        add_css_class: "label-medium-bold",
                        set_halign: gtk::Align::Start,
                        set_label: "Wi-Fi Password (PSK)",
                    },
                    gtk::Label {
                        add_css_class: "label-small",
                        set_halign: gtk::Align::Start,
                        set_xalign: 0.0,
                        set_wrap: true,
                        set_label: "Leave blank to keep the current password unchanged. \
                            WPA2/WPA3 Personal only — enterprise (EAP/PEAP/TTLS) \
                            connections require manual nmcli configuration.",
                    },
                    gtk::Entry {
                        set_hexpand: true,
                        set_visibility: false,
                        set_placeholder_text: Some("New password (optional)"),
                        #[watch]
                        #[block_signal(psk_handler)]
                        set_text: &model.psk,
                        connect_changed[sender] => move |e| {
                            sender.input(ConnectionEditorInput::PskChanged(e.text().to_string()));
                        } @psk_handler,
                    },
                },
            },

            // ── Apply button ──────────────────────────────────────────
            gtk::Button {
                add_css_class: "ok-button-primary",
                set_label: "Apply",
                set_halign: gtk::Align::End,
                set_margin_top: 4,
                #[watch]
                set_sensitive: !model.loading,
                connect_clicked[sender] => move |_| {
                    sender.input(ConnectionEditorInput::Apply);
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        _root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = ConnectionEditorModel::default();
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            // ── Load: trigger background read ─────────────────────────
            ConnectionEditorInput::Load(uuid, name, is_wifi) => {
                // Reset form state, keep identity fields, set loading
                *self = ConnectionEditorModel {
                    uuid: uuid.clone(),
                    conn_name: name,
                    is_wifi,
                    loading: true,
                    ..ConnectionEditorModel::default()
                };

                let sender_c = sender.clone();
                glib::spawn_future_local(async move {
                    let autoconnect = nmcli::get_field(&uuid, "connection.autoconnect")
                        .await
                        .unwrap_or_default()
                        == "yes";
                    let metered = nmcli::get_field(&uuid, "connection.metered")
                        .await
                        .unwrap_or_default();
                    let ipv4_method = nmcli::get_field(&uuid, "ipv4.method")
                        .await
                        .unwrap_or_default();
                    let ipv4_addresses = nmcli::get_field(&uuid, "ipv4.addresses")
                        .await
                        .unwrap_or_default();
                    let ipv4_gateway = nmcli::get_field(&uuid, "ipv4.gateway")
                        .await
                        .unwrap_or_default();
                    let ipv4_dns = nmcli::get_field(&uuid, "ipv4.dns")
                        .await
                        .unwrap_or_default();
                    let ipv4_dns_search = nmcli::get_field(&uuid, "ipv4.dns-search")
                        .await
                        .unwrap_or_default();
                    let ipv4_routes = nmcli::get_field(&uuid, "ipv4.routes")
                        .await
                        .unwrap_or_default();
                    let ipv6_method = nmcli::get_field(&uuid, "ipv6.method")
                        .await
                        .unwrap_or_default();
                    let ipv6_addresses = nmcli::get_field(&uuid, "ipv6.addresses")
                        .await
                        .unwrap_or_default();
                    let ipv6_gateway = nmcli::get_field(&uuid, "ipv6.gateway")
                        .await
                        .unwrap_or_default();
                    let ipv6_dns = nmcli::get_field(&uuid, "ipv6.dns")
                        .await
                        .unwrap_or_default();
                    let ipv6_dns_search = nmcli::get_field(&uuid, "ipv6.dns-search")
                        .await
                        .unwrap_or_default();
                    let ipv6_routes = nmcli::get_field(&uuid, "ipv6.routes")
                        .await
                        .unwrap_or_default();

                    sender_c.input(ConnectionEditorInput::Loaded(EditorFields {
                        autoconnect,
                        metered,
                        ipv4_method,
                        ipv4_addresses,
                        ipv4_gateway,
                        ipv4_dns,
                        ipv4_dns_search,
                        ipv4_routes,
                        ipv6_method,
                        ipv6_addresses,
                        ipv6_gateway,
                        ipv6_dns,
                        ipv6_dns_search,
                        ipv6_routes,
                    }));
                });
            }

            // ── Loaded: populate model from nmcli read ────────────────
            ConnectionEditorInput::Loaded(fields) => {
                self.loading = false;
                self.autoconnect = fields.autoconnect;
                self.metered_idx = metered_index(&fields.metered);
                self.ipv4_method_idx = method_index(IPV4_METHODS, &fields.ipv4_method);
                self.ipv4_addresses = fields.ipv4_addresses;
                self.ipv4_gateway = fields.ipv4_gateway;
                self.ipv4_dns = fields.ipv4_dns;
                self.ipv4_dns_search = fields.ipv4_dns_search;
                self.ipv4_routes = fields.ipv4_routes;
                self.ipv6_method_idx = method_index(IPV6_METHODS, &fields.ipv6_method);
                self.ipv6_addresses = fields.ipv6_addresses;
                self.ipv6_gateway = fields.ipv6_gateway;
                self.ipv6_dns = fields.ipv6_dns;
                self.ipv6_dns_search = fields.ipv6_dns_search;
                self.ipv6_routes = fields.ipv6_routes;
                // PSK is write-only — never read back
                self.psk = String::new();
            }

            // ── Field edits ───────────────────────────────────────────
            ConnectionEditorInput::AutoconnectChanged(v) => {
                self.autoconnect = v;
            }
            ConnectionEditorInput::MeteredChanged(v) => self.metered_idx = v,
            ConnectionEditorInput::Ipv4MethodChanged(v) => self.ipv4_method_idx = v,
            ConnectionEditorInput::Ipv4AddressesChanged(v) => self.ipv4_addresses = v,
            ConnectionEditorInput::Ipv4GatewayChanged(v) => self.ipv4_gateway = v,
            ConnectionEditorInput::Ipv4DnsChanged(v) => self.ipv4_dns = v,
            ConnectionEditorInput::Ipv4DnsSearchChanged(v) => self.ipv4_dns_search = v,
            ConnectionEditorInput::Ipv4RoutesChanged(v) => self.ipv4_routes = v,
            ConnectionEditorInput::Ipv6MethodChanged(v) => self.ipv6_method_idx = v,
            ConnectionEditorInput::Ipv6AddressesChanged(v) => self.ipv6_addresses = v,
            ConnectionEditorInput::Ipv6GatewayChanged(v) => self.ipv6_gateway = v,
            ConnectionEditorInput::Ipv6DnsChanged(v) => self.ipv6_dns = v,
            ConnectionEditorInput::Ipv6DnsSearchChanged(v) => self.ipv6_dns_search = v,
            ConnectionEditorInput::Ipv6RoutesChanged(v) => self.ipv6_routes = v,
            ConnectionEditorInput::PskChanged(v) => self.psk = v,

            // ── Back ──────────────────────────────────────────────────
            ConnectionEditorInput::Back => {
                sender.output(ConnectionEditorOutput::Closed).ok();
            }

            // ── Apply ─────────────────────────────────────────────────
            ConnectionEditorInput::Apply => {
                self.error.clear();

                let ipv4_is_manual =
                    IPV4_METHODS.get(self.ipv4_method_idx as usize) == Some(&"manual");
                let ipv6_is_manual =
                    IPV6_METHODS.get(self.ipv6_method_idx as usize) == Some(&"manual");

                // --- Validation ---
                if ipv4_is_manual {
                    if !self.ipv4_addresses.is_empty() && !cidr_list_valid(&self.ipv4_addresses) {
                        self.error =
                            "IPv4 Addresses: each entry must be in CIDR form (e.g. 192.168.1.50/24)."
                                .to_string();
                        self.update_view(widgets, sender);
                        return;
                    }
                    if !self.ipv4_gateway.is_empty() && self.ipv4_gateway.parse::<IpAddr>().is_err()
                    {
                        self.error = "IPv4 Gateway: not a valid IP address.".to_string();
                        self.update_view(widgets, sender);
                        return;
                    }
                    if !all_ips_valid(&self.ipv4_dns) {
                        self.error = "IPv4 DNS: each entry must be a valid IP address.".to_string();
                        self.update_view(widgets, sender);
                        return;
                    }
                }

                if ipv6_is_manual {
                    if !self.ipv6_addresses.is_empty() && !cidr_list_valid(&self.ipv6_addresses) {
                        self.error =
                            "IPv6 Addresses: each entry must be in CIDR form (e.g. 2001:db8::1/64)."
                                .to_string();
                        self.update_view(widgets, sender);
                        return;
                    }
                    if !self.ipv6_gateway.is_empty() && self.ipv6_gateway.parse::<IpAddr>().is_err()
                    {
                        self.error = "IPv6 Gateway: not a valid IP address.".to_string();
                        self.update_view(widgets, sender);
                        return;
                    }
                    if !all_ips_valid(&self.ipv6_dns) {
                        self.error = "IPv6 DNS: each entry must be a valid IP address.".to_string();
                        self.update_view(widgets, sender);
                        return;
                    }
                }

                // --- Build kv list ---
                // Collect all values into owned Strings first so that
                // the &str borrows in kv are valid for the duration of
                // the async block (borrow-across-await avoided by
                // passing Strings into the async move closure and
                // building kv entirely inside it).
                let autoconnect_val = if self.autoconnect { "yes" } else { "no" }.to_string();
                let metered_val = metered_from_index(self.metered_idx).to_string();
                let ipv4_method_val = IPV4_METHODS
                    .get(self.ipv4_method_idx as usize)
                    .copied()
                    .unwrap_or("auto")
                    .to_string();
                let ipv4_addresses_val = self.ipv4_addresses.clone();
                let ipv4_gateway_val = self.ipv4_gateway.clone();
                let ipv4_dns_val = self.ipv4_dns.clone();
                let ipv4_dns_search_val = self.ipv4_dns_search.clone();
                let ipv4_routes_val = self.ipv4_routes.clone();
                let ipv6_method_val = IPV6_METHODS
                    .get(self.ipv6_method_idx as usize)
                    .copied()
                    .unwrap_or("auto")
                    .to_string();
                let ipv6_addresses_val = self.ipv6_addresses.clone();
                let ipv6_gateway_val = self.ipv6_gateway.clone();
                let ipv6_dns_val = self.ipv6_dns.clone();
                let ipv6_dns_search_val = self.ipv6_dns_search.clone();
                let ipv6_routes_val = self.ipv6_routes.clone();
                let psk_val = self.psk.clone();
                let is_wifi = self.is_wifi;
                let uuid = self.uuid.clone();

                self.loading = true;

                let sender_c = sender.clone();
                glib::spawn_future_local(async move {
                    // Build owned (String, String) pairs — all owned within the
                    // async move block so &str refs are trivially valid.
                    let mut pairs: Vec<(String, String)> = vec![
                        ("connection.autoconnect".into(), autoconnect_val),
                        ("connection.metered".into(), metered_val),
                        ("ipv4.method".into(), ipv4_method_val.clone()),
                        ("ipv6.method".into(), ipv6_method_val.clone()),
                    ];

                    if ipv4_method_val == "manual" {
                        if !ipv4_addresses_val.is_empty() {
                            pairs.push(("ipv4.addresses".into(), ipv4_addresses_val));
                        }
                        if !ipv4_gateway_val.is_empty() {
                            pairs.push(("ipv4.gateway".into(), ipv4_gateway_val));
                        }
                        if !ipv4_dns_val.is_empty() {
                            pairs.push(("ipv4.dns".into(), ipv4_dns_val));
                        }
                        if !ipv4_dns_search_val.is_empty() {
                            pairs.push(("ipv4.dns-search".into(), ipv4_dns_search_val));
                        }
                        if !ipv4_routes_val.is_empty() {
                            pairs.push(("ipv4.routes".into(), ipv4_routes_val));
                        }
                    }

                    if ipv6_method_val == "manual" {
                        if !ipv6_addresses_val.is_empty() {
                            pairs.push(("ipv6.addresses".into(), ipv6_addresses_val));
                        }
                        if !ipv6_gateway_val.is_empty() {
                            pairs.push(("ipv6.gateway".into(), ipv6_gateway_val));
                        }
                        if !ipv6_dns_val.is_empty() {
                            pairs.push(("ipv6.dns".into(), ipv6_dns_val));
                        }
                        if !ipv6_dns_search_val.is_empty() {
                            pairs.push(("ipv6.dns-search".into(), ipv6_dns_search_val));
                        }
                        if !ipv6_routes_val.is_empty() {
                            pairs.push(("ipv6.routes".into(), ipv6_routes_val));
                        }
                    }

                    if is_wifi && !psk_val.is_empty() {
                        pairs.push(("802-11-wireless-security.psk".into(), psk_val));
                    }

                    // nmcli::modify takes &[(&str, &str)] — build refs into
                    // the owned pairs vec, which lives in scope until `.await` returns.
                    let kv: Vec<(&str, &str)> = pairs
                        .iter()
                        .map(|(k, v)| (k.as_str(), v.as_str()))
                        .collect();

                    if let Err(e) = nmcli::modify(&uuid, &kv).await {
                        mshell_launcher::notify::toast("Network", &e);
                        // Send a no-op Loaded so the spinner stops; the user
                        // can fix and retry.
                        sender_c.input(ConnectionEditorInput::Loaded(EditorFields::default()));
                        return;
                    }

                    // Best-effort reconnect — ignore errors (VPN / offline).
                    let _ = nmcli::up(&uuid).await;

                    sender_c.output(ConnectionEditorOutput::Closed).ok();
                });
            }
        }

        self.update_view(widgets, sender);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dns_list_ok() {
        assert!(all_ips_valid("1.1.1.1 8.8.8.8"));
        assert!(all_ips_valid("1.1.1.1, 9.9.9.9"));
    }

    #[test]
    fn dns_list_bad() {
        assert!(!all_ips_valid("1.1.1.1 not-an-ip"));
    }

    #[test]
    fn dns_empty_ok() {
        // empty = valid (means "leave unset")
        assert!(all_ips_valid(""));
    }

    #[test]
    fn cidr_ok() {
        assert!(cidr_valid("192.168.1.50/24"));
    }

    #[test]
    fn cidr_bad() {
        assert!(!cidr_valid("192.168.1.50")); // no prefix
        assert!(!cidr_valid("999.0.0.0/24")); // invalid IP
    }

    #[test]
    fn ipv6_cidr_ok() {
        assert!(cidr_valid("2001:db8::1/64"));
        assert!(all_ips_valid("2606:4700:4700::1111 2606:4700:4700::1001"));
    }

    #[test]
    fn metered_roundtrip() {
        assert_eq!(metered_from_index(metered_index("yes")), "yes");
        assert_eq!(metered_from_index(metered_index("no")), "no");
        assert_eq!(metered_from_index(metered_index("unknown")), "unknown");
        assert_eq!(metered_from_index(metered_index("")), "unknown");
    }

    #[test]
    fn method_index_roundtrip() {
        for (i, &m) in IPV4_METHODS.iter().enumerate() {
            assert_eq!(method_index(IPV4_METHODS, m), i as u32);
        }
    }

    #[test]
    fn cidr_list_multiple_ok() {
        assert!(cidr_list_valid("192.168.1.0/24 10.0.0.1/8"));
        assert!(cidr_list_valid("192.168.1.0/24, 10.0.0.1/8"));
    }

    #[test]
    fn cidr_list_bad() {
        assert!(!cidr_list_valid("192.168.1.0/24 10.0.0.1")); // missing prefix
    }
}
