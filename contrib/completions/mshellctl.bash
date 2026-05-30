# bash completion for mshellctl
# vim: ft=bash
#
# Install:
#   mkdir -p ~/.local/share/bash-completion/completions
#   cp contrib/completions/mshellctl.bash ~/.local/share/bash-completion/completions/mshellctl
#
# Completes mshellctl's static subcommand surface plus dynamic lookups for
# plugin keys (used by `plugin reload / keybind` and `menu plugin`). The
# plugin list is pulled from `mshellctl plugin list --names` at completion
# time so newly installed plugins show up without a re-source.

_mshellctl() {
    local cur prev words cword
    _init_completion -n =: 2>/dev/null || {
        cur="${COMP_WORDS[COMP_CWORD]}"
        prev="${COMP_WORDS[COMP_CWORD - 1]}"
        words=("${COMP_WORDS[@]}")
        cword=$COMP_CWORD
    }

    local top_subs=(
        audio bar brightness inspect lock media menu plugin quit screenshot
        set-wallpaper settings wallpaper wizard help
    )

    # Pull plugin keys live; falls back silently if mshellctl can't reach
    # the local plugin store (e.g. running on a different machine).
    _mshellctl_plugin_names() {
        mshellctl plugin list --names 2>/dev/null
    }

    # Resolve the active subcommand by scanning past global flags.
    local sub=""
    local sub2=""
    local sub3=""
    local i=1
    while [ $i -lt "$cword" ]; do
        case "${words[$i]}" in
            -*) i=$((i + 1)) ;;
            *)
                if [ -z "$sub" ]; then
                    sub="${words[$i]}"
                elif [ -z "$sub2" ]; then
                    sub2="${words[$i]}"
                elif [ -z "$sub3" ]; then
                    sub3="${words[$i]}"
                else
                    break
                fi
                i=$((i + 1))
                ;;
        esac
    done

    # Top level.
    if [ -z "$sub" ]; then
        COMPREPLY=($(compgen -W "${top_subs[*]}" -- "$cur"))
        return 0
    fi

    case "$sub" in
        menu)
            # `mshellctl menu <COMMAND>`
            local menu_subs=(
                app-launcher clipboard clock notifications screenshot wallpaper
                ufw bluetooth cpu-dashboard audio-dashboard system-update valent
                keep-awake twilight weather keybinds alarm-clock control-center
                ssh-sessions dns podman notes ip network power media-player
                session dashboard mshelldash plugin
            )
            if [ -z "$sub2" ]; then
                COMPREPLY=($(compgen -W "${menu_subs[*]}" -- "$cur"))
                return 0
            fi
            # `mshellctl menu plugin <KEY>`
            if [ "$sub2" = "plugin" ] && [ -z "$sub3" ]; then
                local keys
                keys=$(_mshellctl_plugin_names)
                COMPREPLY=($(compgen -W "$keys" -- "$cur"))
                return 0
            fi
            ;;
        plugin)
            # `mshellctl plugin <COMMAND>`
            local plugin_subs=(list reload keybind help)
            if [ -z "$sub2" ]; then
                COMPREPLY=($(compgen -W "${plugin_subs[*]}" -- "$cur"))
                return 0
            fi
            # `mshellctl plugin reload <KEY>` / `keybind <KEY>`
            if { [ "$sub2" = "reload" ] || [ "$sub2" = "keybind" ]; } && [ -z "$sub3" ]; then
                local keys
                keys=$(_mshellctl_plugin_names)
                COMPREPLY=($(compgen -W "$keys" -- "$cur"))
                return 0
            fi
            if [ "$sub2" = "list" ]; then
                COMPREPLY=($(compgen -W "--names --enabled" -- "$cur"))
                return 0
            fi
            ;;
        audio)
            COMPREPLY=($(compgen -W "volume mute toggle-mute output input help" -- "$cur"))
            return 0
            ;;
        bar)
            COMPREPLY=($(compgen -W "show hide toggle help" -- "$cur"))
            return 0
            ;;
        brightness)
            COMPREPLY=($(compgen -W "set up down help" -- "$cur"))
            return 0
            ;;
        wallpaper)
            COMPREPLY=($(compgen -W "next prev random help" -- "$cur"))
            return 0
            ;;
        settings)
            COMPREPLY=($(compgen -W "open close toggle help" -- "$cur"))
            return 0
            ;;
        screenshot)
            COMPREPLY=($(compgen -W "select-region help" -- "$cur"))
            return 0
            ;;
        lock)
            COMPREPLY=($(compgen -W "check help" -- "$cur"))
            return 0
            ;;
        set-wallpaper)
            # Path completion.
            COMPREPLY=($(compgen -f -- "$cur"))
            return 0
            ;;
    esac
}

complete -F _mshellctl mshellctl
