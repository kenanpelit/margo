# fish completion for mshellctl
#
# Install:
#   mkdir -p ~/.config/fish/completions
#   cp contrib/completions/mshellctl.fish ~/.config/fish/completions/mshellctl.fish
#
# Plugin keys come live from `mshellctl plugin list --names`, so newly
# installed plugins show up without re-sourcing.

set -l top_subs audio bar brightness inspect lock media menu plugin quit screenshot set-wallpaper settings wallpaper wizard help

set -l menu_subs app-launcher clipboard clock notifications screenshot wallpaper ufw bluetooth cpu-dashboard audio-dashboard system-update valent keep-awake twilight weather keybinds alarm-clock control-center ssh-sessions dns podman notes ip network power media-player session dashboard plugin

set -l plugin_subs list reload keybind help

# Helper: are we on the top level (no subcommand chosen yet)?
function __mshellctl_no_sub
    set -l tokens (commandline -opc)
    set -l subs audio bar brightness inspect lock media menu plugin quit screenshot set-wallpaper settings wallpaper wizard help
    for tok in $tokens[2..-1]
        if contains -- $tok $subs
            return 1
        end
    end
    return 0
end

function __mshellctl_sub_is -a sub
    set -l tokens (commandline -opc)
    set -l subs audio bar brightness inspect lock media menu plugin quit screenshot set-wallpaper settings wallpaper wizard help
    for tok in $tokens[2..-1]
        if contains -- $tok $subs
            test "$tok" = "$sub"
            return $status
        end
    end
    return 1
end

function __mshellctl_menu_sub_is -a sub
    set -l tokens (commandline -opc)
    set -l in_menu 0
    for tok in $tokens[2..-1]
        if test $in_menu -eq 1
            test "$tok" = "$sub"
            return $status
        end
        if test "$tok" = "menu"
            set in_menu 1
        end
    end
    return 1
end

function __mshellctl_plugin_sub_is -a sub
    set -l tokens (commandline -opc)
    set -l in_plugin 0
    for tok in $tokens[2..-1]
        if test $in_plugin -eq 1
            test "$tok" = "$sub"
            return $status
        end
        if test "$tok" = "plugin"
            set in_plugin 1
        end
    end
    return 1
end

function __mshellctl_plugin_names
    mshellctl plugin list --names 2>/dev/null
end

# Top level
complete -c mshellctl -n __mshellctl_no_sub -a "$top_subs" -f

# menu <sub>
complete -c mshellctl -n "__mshellctl_sub_is menu" -a "$menu_subs" -f
# menu plugin <key>
complete -c mshellctl -n "__mshellctl_menu_sub_is plugin" -a "(__mshellctl_plugin_names)" -f

# plugin <sub>
complete -c mshellctl -n "__mshellctl_sub_is plugin" -a "$plugin_subs" -f
# plugin reload <key>
complete -c mshellctl -n "__mshellctl_plugin_sub_is reload" -a "(__mshellctl_plugin_names)" -f
# plugin keybind <key>
complete -c mshellctl -n "__mshellctl_plugin_sub_is keybind" -a "(__mshellctl_plugin_names)" -f
# plugin list flags
complete -c mshellctl -n "__mshellctl_plugin_sub_is list" -l names -d "Print only composite keys"
complete -c mshellctl -n "__mshellctl_plugin_sub_is list" -l enabled -d "Only show enabled plugins"

# Subcommand-specific shallow lists
complete -c mshellctl -n "__mshellctl_sub_is audio" -a "volume mute toggle-mute output input help" -f
complete -c mshellctl -n "__mshellctl_sub_is bar" -a "show hide toggle help" -f
complete -c mshellctl -n "__mshellctl_sub_is brightness" -a "set up down help" -f
complete -c mshellctl -n "__mshellctl_sub_is wallpaper" -a "next prev random help" -f
complete -c mshellctl -n "__mshellctl_sub_is settings" -a "open close toggle help" -f
complete -c mshellctl -n "__mshellctl_sub_is screenshot" -a "select-region help" -f
complete -c mshellctl -n "__mshellctl_sub_is lock" -a "check help" -f
complete -c mshellctl -n "__mshellctl_sub_is set-wallpaper" -F
