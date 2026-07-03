_mshellctl() {
    local i cur prev opts cmd
    COMPREPLY=()
    if [[ "${BASH_VERSINFO[0]}" -ge 4 ]]; then
        cur="$2"
    else
        cur="${COMP_WORDS[COMP_CWORD]}"
    fi
    prev="$3"
    cmd=""
    opts=""

    for i in "${COMP_WORDS[@]:0:COMP_CWORD}"
    do
        case "${cmd},${i}" in
            ",$1")
                cmd="mshellctl"
                ;;
            mshellctl,audio)
                cmd="mshellctl__subcmd__audio"
                ;;
            mshellctl,bar)
                cmd="mshellctl__subcmd__bar"
                ;;
            mshellctl,bluetooth)
                cmd="mshellctl__subcmd__bluetooth"
                ;;
            mshellctl,brightness)
                cmd="mshellctl__subcmd__brightness"
                ;;
            mshellctl,calendar)
                cmd="mshellctl__subcmd__calendar"
                ;;
            mshellctl,clipboard)
                cmd="mshellctl__subcmd__clipboard"
                ;;
            mshellctl,color)
                cmd="mshellctl__subcmd__color"
                ;;
            mshellctl,completions)
                cmd="mshellctl__subcmd__completions"
                ;;
            mshellctl,dock)
                cmd="mshellctl__subcmd__dock"
                ;;
            mshellctl,doctor)
                cmd="mshellctl__subcmd__doctor"
                ;;
            mshellctl,gamemode)
                cmd="mshellctl__subcmd__gamemode"
                ;;
            mshellctl,help)
                cmd="mshellctl__subcmd__help"
                ;;
            mshellctl,hidden-bar)
                cmd="mshellctl__subcmd__hidden__subcmd__bar"
                ;;
            mshellctl,inspect)
                cmd="mshellctl__subcmd__inspect"
                ;;
            mshellctl,layout)
                cmd="mshellctl__subcmd__layout"
                ;;
            mshellctl,lock)
                cmd="mshellctl__subcmd__lock"
                ;;
            mshellctl,log)
                cmd="mshellctl__subcmd__log"
                ;;
            mshellctl,media)
                cmd="mshellctl__subcmd__media"
                ;;
            mshellctl,menu)
                cmd="mshellctl__subcmd__menu"
                ;;
            mshellctl,notification)
                cmd="mshellctl__subcmd__notification"
                ;;
            mshellctl,osk)
                cmd="mshellctl__subcmd__osk"
                ;;
            mshellctl,play)
                cmd="mshellctl__subcmd__play"
                ;;
            mshellctl,plugin)
                cmd="mshellctl__subcmd__plugin"
                ;;
            mshellctl,power)
                cmd="mshellctl__subcmd__power"
                ;;
            mshellctl,quit)
                cmd="mshellctl__subcmd__quit"
                ;;
            mshellctl,screenrecord)
                cmd="mshellctl__subcmd__screenrecord"
                ;;
            mshellctl,screenshot)
                cmd="mshellctl__subcmd__screenshot"
                ;;
            mshellctl,session)
                cmd="mshellctl__subcmd__session"
                ;;
            mshellctl,set-wallpaper)
                cmd="mshellctl__subcmd__set__subcmd__wallpaper"
                ;;
            mshellctl,settings)
                cmd="mshellctl__subcmd__settings"
                ;;
            mshellctl,theme)
                cmd="mshellctl__subcmd__theme"
                ;;
            mshellctl,toast)
                cmd="mshellctl__subcmd__toast"
                ;;
            mshellctl,vpn)
                cmd="mshellctl__subcmd__vpn"
                ;;
            mshellctl,wallpaper)
                cmd="mshellctl__subcmd__wallpaper"
                ;;
            mshellctl,wizard)
                cmd="mshellctl__subcmd__wizard"
                ;;
            mshellctl__subcmd__audio,help)
                cmd="mshellctl__subcmd__audio__subcmd__help"
                ;;
            mshellctl__subcmd__audio,input)
                cmd="mshellctl__subcmd__audio__subcmd__input"
                ;;
            mshellctl__subcmd__audio,list)
                cmd="mshellctl__subcmd__audio__subcmd__list"
                ;;
            mshellctl__subcmd__audio,mic)
                cmd="mshellctl__subcmd__audio__subcmd__mic"
                ;;
            mshellctl__subcmd__audio,mic-down)
                cmd="mshellctl__subcmd__audio__subcmd__mic__subcmd__down"
                ;;
            mshellctl__subcmd__audio,mic-mute)
                cmd="mshellctl__subcmd__audio__subcmd__mic__subcmd__mute"
                ;;
            mshellctl__subcmd__audio,mic-up)
                cmd="mshellctl__subcmd__audio__subcmd__mic__subcmd__up"
                ;;
            mshellctl__subcmd__audio,mute)
                cmd="mshellctl__subcmd__audio__subcmd__mute"
                ;;
            mshellctl__subcmd__audio,output)
                cmd="mshellctl__subcmd__audio__subcmd__output"
                ;;
            mshellctl__subcmd__audio,route-next)
                cmd="mshellctl__subcmd__audio__subcmd__route__subcmd__next"
                ;;
            mshellctl__subcmd__audio,status)
                cmd="mshellctl__subcmd__audio__subcmd__status"
                ;;
            mshellctl__subcmd__audio,switch)
                cmd="mshellctl__subcmd__audio__subcmd__switch"
                ;;
            mshellctl__subcmd__audio,switch-mic)
                cmd="mshellctl__subcmd__audio__subcmd__switch__subcmd__mic"
                ;;
            mshellctl__subcmd__audio,volume)
                cmd="mshellctl__subcmd__audio__subcmd__volume"
                ;;
            mshellctl__subcmd__audio,volume-down)
                cmd="mshellctl__subcmd__audio__subcmd__volume__subcmd__down"
                ;;
            mshellctl__subcmd__audio,volume-up)
                cmd="mshellctl__subcmd__audio__subcmd__volume__subcmd__up"
                ;;
            mshellctl__subcmd__audio__subcmd__help,help)
                cmd="mshellctl__subcmd__audio__subcmd__help__subcmd__help"
                ;;
            mshellctl__subcmd__audio__subcmd__help,input)
                cmd="mshellctl__subcmd__audio__subcmd__help__subcmd__input"
                ;;
            mshellctl__subcmd__audio__subcmd__help,list)
                cmd="mshellctl__subcmd__audio__subcmd__help__subcmd__list"
                ;;
            mshellctl__subcmd__audio__subcmd__help,mic)
                cmd="mshellctl__subcmd__audio__subcmd__help__subcmd__mic"
                ;;
            mshellctl__subcmd__audio__subcmd__help,mic-down)
                cmd="mshellctl__subcmd__audio__subcmd__help__subcmd__mic__subcmd__down"
                ;;
            mshellctl__subcmd__audio__subcmd__help,mic-mute)
                cmd="mshellctl__subcmd__audio__subcmd__help__subcmd__mic__subcmd__mute"
                ;;
            mshellctl__subcmd__audio__subcmd__help,mic-up)
                cmd="mshellctl__subcmd__audio__subcmd__help__subcmd__mic__subcmd__up"
                ;;
            mshellctl__subcmd__audio__subcmd__help,mute)
                cmd="mshellctl__subcmd__audio__subcmd__help__subcmd__mute"
                ;;
            mshellctl__subcmd__audio__subcmd__help,output)
                cmd="mshellctl__subcmd__audio__subcmd__help__subcmd__output"
                ;;
            mshellctl__subcmd__audio__subcmd__help,route-next)
                cmd="mshellctl__subcmd__audio__subcmd__help__subcmd__route__subcmd__next"
                ;;
            mshellctl__subcmd__audio__subcmd__help,status)
                cmd="mshellctl__subcmd__audio__subcmd__help__subcmd__status"
                ;;
            mshellctl__subcmd__audio__subcmd__help,switch)
                cmd="mshellctl__subcmd__audio__subcmd__help__subcmd__switch"
                ;;
            mshellctl__subcmd__audio__subcmd__help,switch-mic)
                cmd="mshellctl__subcmd__audio__subcmd__help__subcmd__switch__subcmd__mic"
                ;;
            mshellctl__subcmd__audio__subcmd__help,volume)
                cmd="mshellctl__subcmd__audio__subcmd__help__subcmd__volume"
                ;;
            mshellctl__subcmd__audio__subcmd__help,volume-down)
                cmd="mshellctl__subcmd__audio__subcmd__help__subcmd__volume__subcmd__down"
                ;;
            mshellctl__subcmd__audio__subcmd__help,volume-up)
                cmd="mshellctl__subcmd__audio__subcmd__help__subcmd__volume__subcmd__up"
                ;;
            mshellctl__subcmd__bar,bottom)
                cmd="mshellctl__subcmd__bar__subcmd__bottom"
                ;;
            mshellctl__subcmd__bar,help)
                cmd="mshellctl__subcmd__bar__subcmd__help"
                ;;
            mshellctl__subcmd__bar,hide)
                cmd="mshellctl__subcmd__bar__subcmd__hide__subcmd__all"
                ;;
            mshellctl__subcmd__bar,hide-all)
                cmd="mshellctl__subcmd__bar__subcmd__hide__subcmd__all"
                ;;
            mshellctl__subcmd__bar,reveal)
                cmd="mshellctl__subcmd__bar__subcmd__reveal__subcmd__all"
                ;;
            mshellctl__subcmd__bar,reveal-all)
                cmd="mshellctl__subcmd__bar__subcmd__reveal__subcmd__all"
                ;;
            mshellctl__subcmd__bar,show)
                cmd="mshellctl__subcmd__bar__subcmd__reveal__subcmd__all"
                ;;
            mshellctl__subcmd__bar,show-all)
                cmd="mshellctl__subcmd__bar__subcmd__reveal__subcmd__all"
                ;;
            mshellctl__subcmd__bar,toggle)
                cmd="mshellctl__subcmd__bar__subcmd__toggle__subcmd__all"
                ;;
            mshellctl__subcmd__bar,toggle-all)
                cmd="mshellctl__subcmd__bar__subcmd__toggle__subcmd__all"
                ;;
            mshellctl__subcmd__bar,top)
                cmd="mshellctl__subcmd__bar__subcmd__top"
                ;;
            mshellctl__subcmd__bar__subcmd__help,bottom)
                cmd="mshellctl__subcmd__bar__subcmd__help__subcmd__bottom"
                ;;
            mshellctl__subcmd__bar__subcmd__help,help)
                cmd="mshellctl__subcmd__bar__subcmd__help__subcmd__help"
                ;;
            mshellctl__subcmd__bar__subcmd__help,hide-all)
                cmd="mshellctl__subcmd__bar__subcmd__help__subcmd__hide__subcmd__all"
                ;;
            mshellctl__subcmd__bar__subcmd__help,reveal-all)
                cmd="mshellctl__subcmd__bar__subcmd__help__subcmd__reveal__subcmd__all"
                ;;
            mshellctl__subcmd__bar__subcmd__help,toggle-all)
                cmd="mshellctl__subcmd__bar__subcmd__help__subcmd__toggle__subcmd__all"
                ;;
            mshellctl__subcmd__bar__subcmd__help,top)
                cmd="mshellctl__subcmd__bar__subcmd__help__subcmd__top"
                ;;
            mshellctl__subcmd__bluetooth,connect)
                cmd="mshellctl__subcmd__bluetooth__subcmd__connect"
                ;;
            mshellctl__subcmd__bluetooth,disconnect)
                cmd="mshellctl__subcmd__bluetooth__subcmd__disconnect"
                ;;
            mshellctl__subcmd__bluetooth,help)
                cmd="mshellctl__subcmd__bluetooth__subcmd__help"
                ;;
            mshellctl__subcmd__bluetooth,toggle)
                cmd="mshellctl__subcmd__bluetooth__subcmd__toggle"
                ;;
            mshellctl__subcmd__bluetooth__subcmd__help,connect)
                cmd="mshellctl__subcmd__bluetooth__subcmd__help__subcmd__connect"
                ;;
            mshellctl__subcmd__bluetooth__subcmd__help,disconnect)
                cmd="mshellctl__subcmd__bluetooth__subcmd__help__subcmd__disconnect"
                ;;
            mshellctl__subcmd__bluetooth__subcmd__help,help)
                cmd="mshellctl__subcmd__bluetooth__subcmd__help__subcmd__help"
                ;;
            mshellctl__subcmd__bluetooth__subcmd__help,toggle)
                cmd="mshellctl__subcmd__bluetooth__subcmd__help__subcmd__toggle"
                ;;
            mshellctl__subcmd__brightness,down)
                cmd="mshellctl__subcmd__brightness__subcmd__down"
                ;;
            mshellctl__subcmd__brightness,help)
                cmd="mshellctl__subcmd__brightness__subcmd__help"
                ;;
            mshellctl__subcmd__brightness,up)
                cmd="mshellctl__subcmd__brightness__subcmd__up"
                ;;
            mshellctl__subcmd__brightness__subcmd__help,down)
                cmd="mshellctl__subcmd__brightness__subcmd__help__subcmd__down"
                ;;
            mshellctl__subcmd__brightness__subcmd__help,help)
                cmd="mshellctl__subcmd__brightness__subcmd__help__subcmd__help"
                ;;
            mshellctl__subcmd__brightness__subcmd__help,up)
                cmd="mshellctl__subcmd__brightness__subcmd__help__subcmd__up"
                ;;
            mshellctl__subcmd__calendar,account)
                cmd="mshellctl__subcmd__calendar__subcmd__account"
                ;;
            mshellctl__subcmd__calendar,agenda)
                cmd="mshellctl__subcmd__calendar__subcmd__agenda"
                ;;
            mshellctl__subcmd__calendar,help)
                cmd="mshellctl__subcmd__calendar__subcmd__help"
                ;;
            mshellctl__subcmd__calendar,on)
                cmd="mshellctl__subcmd__calendar__subcmd__on"
                ;;
            mshellctl__subcmd__calendar,today)
                cmd="mshellctl__subcmd__calendar__subcmd__today"
                ;;
            mshellctl__subcmd__calendar__subcmd__help,account)
                cmd="mshellctl__subcmd__calendar__subcmd__help__subcmd__account"
                ;;
            mshellctl__subcmd__calendar__subcmd__help,agenda)
                cmd="mshellctl__subcmd__calendar__subcmd__help__subcmd__agenda"
                ;;
            mshellctl__subcmd__calendar__subcmd__help,help)
                cmd="mshellctl__subcmd__calendar__subcmd__help__subcmd__help"
                ;;
            mshellctl__subcmd__calendar__subcmd__help,on)
                cmd="mshellctl__subcmd__calendar__subcmd__help__subcmd__on"
                ;;
            mshellctl__subcmd__calendar__subcmd__help,today)
                cmd="mshellctl__subcmd__calendar__subcmd__help__subcmd__today"
                ;;
            mshellctl__subcmd__clipboard,clear)
                cmd="mshellctl__subcmd__clipboard__subcmd__clear"
                ;;
            mshellctl__subcmd__clipboard,copy)
                cmd="mshellctl__subcmd__clipboard__subcmd__copy"
                ;;
            mshellctl__subcmd__clipboard,delete)
                cmd="mshellctl__subcmd__clipboard__subcmd__delete"
                ;;
            mshellctl__subcmd__clipboard,help)
                cmd="mshellctl__subcmd__clipboard__subcmd__help"
                ;;
            mshellctl__subcmd__clipboard,list)
                cmd="mshellctl__subcmd__clipboard__subcmd__list"
                ;;
            mshellctl__subcmd__clipboard,pin)
                cmd="mshellctl__subcmd__clipboard__subcmd__pin"
                ;;
            mshellctl__subcmd__clipboard,unpin)
                cmd="mshellctl__subcmd__clipboard__subcmd__unpin"
                ;;
            mshellctl__subcmd__clipboard,wipe)
                cmd="mshellctl__subcmd__clipboard__subcmd__wipe"
                ;;
            mshellctl__subcmd__clipboard__subcmd__help,clear)
                cmd="mshellctl__subcmd__clipboard__subcmd__help__subcmd__clear"
                ;;
            mshellctl__subcmd__clipboard__subcmd__help,copy)
                cmd="mshellctl__subcmd__clipboard__subcmd__help__subcmd__copy"
                ;;
            mshellctl__subcmd__clipboard__subcmd__help,delete)
                cmd="mshellctl__subcmd__clipboard__subcmd__help__subcmd__delete"
                ;;
            mshellctl__subcmd__clipboard__subcmd__help,help)
                cmd="mshellctl__subcmd__clipboard__subcmd__help__subcmd__help"
                ;;
            mshellctl__subcmd__clipboard__subcmd__help,list)
                cmd="mshellctl__subcmd__clipboard__subcmd__help__subcmd__list"
                ;;
            mshellctl__subcmd__clipboard__subcmd__help,pin)
                cmd="mshellctl__subcmd__clipboard__subcmd__help__subcmd__pin"
                ;;
            mshellctl__subcmd__clipboard__subcmd__help,unpin)
                cmd="mshellctl__subcmd__clipboard__subcmd__help__subcmd__unpin"
                ;;
            mshellctl__subcmd__clipboard__subcmd__help,wipe)
                cmd="mshellctl__subcmd__clipboard__subcmd__help__subcmd__wipe"
                ;;
            mshellctl__subcmd__dock,activate)
                cmd="mshellctl__subcmd__dock__subcmd__activate"
                ;;
            mshellctl__subcmd__dock,help)
                cmd="mshellctl__subcmd__dock__subcmd__help"
                ;;
            mshellctl__subcmd__dock,hide)
                cmd="mshellctl__subcmd__dock__subcmd__hide"
                ;;
            mshellctl__subcmd__dock,show)
                cmd="mshellctl__subcmd__dock__subcmd__show"
                ;;
            mshellctl__subcmd__dock,toggle)
                cmd="mshellctl__subcmd__dock__subcmd__toggle"
                ;;
            mshellctl__subcmd__dock__subcmd__help,activate)
                cmd="mshellctl__subcmd__dock__subcmd__help__subcmd__activate"
                ;;
            mshellctl__subcmd__dock__subcmd__help,help)
                cmd="mshellctl__subcmd__dock__subcmd__help__subcmd__help"
                ;;
            mshellctl__subcmd__dock__subcmd__help,hide)
                cmd="mshellctl__subcmd__dock__subcmd__help__subcmd__hide"
                ;;
            mshellctl__subcmd__dock__subcmd__help,show)
                cmd="mshellctl__subcmd__dock__subcmd__help__subcmd__show"
                ;;
            mshellctl__subcmd__dock__subcmd__help,toggle)
                cmd="mshellctl__subcmd__dock__subcmd__help__subcmd__toggle"
                ;;
            mshellctl__subcmd__help,audio)
                cmd="mshellctl__subcmd__help__subcmd__audio"
                ;;
            mshellctl__subcmd__help,bar)
                cmd="mshellctl__subcmd__help__subcmd__bar"
                ;;
            mshellctl__subcmd__help,bluetooth)
                cmd="mshellctl__subcmd__help__subcmd__bluetooth"
                ;;
            mshellctl__subcmd__help,brightness)
                cmd="mshellctl__subcmd__help__subcmd__brightness"
                ;;
            mshellctl__subcmd__help,calendar)
                cmd="mshellctl__subcmd__help__subcmd__calendar"
                ;;
            mshellctl__subcmd__help,clipboard)
                cmd="mshellctl__subcmd__help__subcmd__clipboard"
                ;;
            mshellctl__subcmd__help,color)
                cmd="mshellctl__subcmd__help__subcmd__color"
                ;;
            mshellctl__subcmd__help,completions)
                cmd="mshellctl__subcmd__help__subcmd__completions"
                ;;
            mshellctl__subcmd__help,dock)
                cmd="mshellctl__subcmd__help__subcmd__dock"
                ;;
            mshellctl__subcmd__help,doctor)
                cmd="mshellctl__subcmd__help__subcmd__doctor"
                ;;
            mshellctl__subcmd__help,gamemode)
                cmd="mshellctl__subcmd__help__subcmd__gamemode"
                ;;
            mshellctl__subcmd__help,help)
                cmd="mshellctl__subcmd__help__subcmd__help"
                ;;
            mshellctl__subcmd__help,hidden-bar)
                cmd="mshellctl__subcmd__help__subcmd__hidden__subcmd__bar"
                ;;
            mshellctl__subcmd__help,inspect)
                cmd="mshellctl__subcmd__help__subcmd__inspect"
                ;;
            mshellctl__subcmd__help,layout)
                cmd="mshellctl__subcmd__help__subcmd__layout"
                ;;
            mshellctl__subcmd__help,lock)
                cmd="mshellctl__subcmd__help__subcmd__lock"
                ;;
            mshellctl__subcmd__help,log)
                cmd="mshellctl__subcmd__help__subcmd__log"
                ;;
            mshellctl__subcmd__help,media)
                cmd="mshellctl__subcmd__help__subcmd__media"
                ;;
            mshellctl__subcmd__help,menu)
                cmd="mshellctl__subcmd__help__subcmd__menu"
                ;;
            mshellctl__subcmd__help,notification)
                cmd="mshellctl__subcmd__help__subcmd__notification"
                ;;
            mshellctl__subcmd__help,osk)
                cmd="mshellctl__subcmd__help__subcmd__osk"
                ;;
            mshellctl__subcmd__help,play)
                cmd="mshellctl__subcmd__help__subcmd__play"
                ;;
            mshellctl__subcmd__help,plugin)
                cmd="mshellctl__subcmd__help__subcmd__plugin"
                ;;
            mshellctl__subcmd__help,power)
                cmd="mshellctl__subcmd__help__subcmd__power"
                ;;
            mshellctl__subcmd__help,quit)
                cmd="mshellctl__subcmd__help__subcmd__quit"
                ;;
            mshellctl__subcmd__help,screenrecord)
                cmd="mshellctl__subcmd__help__subcmd__screenrecord"
                ;;
            mshellctl__subcmd__help,screenshot)
                cmd="mshellctl__subcmd__help__subcmd__screenshot"
                ;;
            mshellctl__subcmd__help,session)
                cmd="mshellctl__subcmd__help__subcmd__session"
                ;;
            mshellctl__subcmd__help,set-wallpaper)
                cmd="mshellctl__subcmd__help__subcmd__set__subcmd__wallpaper"
                ;;
            mshellctl__subcmd__help,settings)
                cmd="mshellctl__subcmd__help__subcmd__settings"
                ;;
            mshellctl__subcmd__help,theme)
                cmd="mshellctl__subcmd__help__subcmd__theme"
                ;;
            mshellctl__subcmd__help,toast)
                cmd="mshellctl__subcmd__help__subcmd__toast"
                ;;
            mshellctl__subcmd__help,vpn)
                cmd="mshellctl__subcmd__help__subcmd__vpn"
                ;;
            mshellctl__subcmd__help,wallpaper)
                cmd="mshellctl__subcmd__help__subcmd__wallpaper"
                ;;
            mshellctl__subcmd__help,wizard)
                cmd="mshellctl__subcmd__help__subcmd__wizard"
                ;;
            mshellctl__subcmd__help__subcmd__audio,input)
                cmd="mshellctl__subcmd__help__subcmd__audio__subcmd__input"
                ;;
            mshellctl__subcmd__help__subcmd__audio,list)
                cmd="mshellctl__subcmd__help__subcmd__audio__subcmd__list"
                ;;
            mshellctl__subcmd__help__subcmd__audio,mic)
                cmd="mshellctl__subcmd__help__subcmd__audio__subcmd__mic"
                ;;
            mshellctl__subcmd__help__subcmd__audio,mic-down)
                cmd="mshellctl__subcmd__help__subcmd__audio__subcmd__mic__subcmd__down"
                ;;
            mshellctl__subcmd__help__subcmd__audio,mic-mute)
                cmd="mshellctl__subcmd__help__subcmd__audio__subcmd__mic__subcmd__mute"
                ;;
            mshellctl__subcmd__help__subcmd__audio,mic-up)
                cmd="mshellctl__subcmd__help__subcmd__audio__subcmd__mic__subcmd__up"
                ;;
            mshellctl__subcmd__help__subcmd__audio,mute)
                cmd="mshellctl__subcmd__help__subcmd__audio__subcmd__mute"
                ;;
            mshellctl__subcmd__help__subcmd__audio,output)
                cmd="mshellctl__subcmd__help__subcmd__audio__subcmd__output"
                ;;
            mshellctl__subcmd__help__subcmd__audio,route-next)
                cmd="mshellctl__subcmd__help__subcmd__audio__subcmd__route__subcmd__next"
                ;;
            mshellctl__subcmd__help__subcmd__audio,status)
                cmd="mshellctl__subcmd__help__subcmd__audio__subcmd__status"
                ;;
            mshellctl__subcmd__help__subcmd__audio,switch)
                cmd="mshellctl__subcmd__help__subcmd__audio__subcmd__switch"
                ;;
            mshellctl__subcmd__help__subcmd__audio,switch-mic)
                cmd="mshellctl__subcmd__help__subcmd__audio__subcmd__switch__subcmd__mic"
                ;;
            mshellctl__subcmd__help__subcmd__audio,volume)
                cmd="mshellctl__subcmd__help__subcmd__audio__subcmd__volume"
                ;;
            mshellctl__subcmd__help__subcmd__audio,volume-down)
                cmd="mshellctl__subcmd__help__subcmd__audio__subcmd__volume__subcmd__down"
                ;;
            mshellctl__subcmd__help__subcmd__audio,volume-up)
                cmd="mshellctl__subcmd__help__subcmd__audio__subcmd__volume__subcmd__up"
                ;;
            mshellctl__subcmd__help__subcmd__bar,bottom)
                cmd="mshellctl__subcmd__help__subcmd__bar__subcmd__bottom"
                ;;
            mshellctl__subcmd__help__subcmd__bar,hide-all)
                cmd="mshellctl__subcmd__help__subcmd__bar__subcmd__hide__subcmd__all"
                ;;
            mshellctl__subcmd__help__subcmd__bar,reveal-all)
                cmd="mshellctl__subcmd__help__subcmd__bar__subcmd__reveal__subcmd__all"
                ;;
            mshellctl__subcmd__help__subcmd__bar,toggle-all)
                cmd="mshellctl__subcmd__help__subcmd__bar__subcmd__toggle__subcmd__all"
                ;;
            mshellctl__subcmd__help__subcmd__bar,top)
                cmd="mshellctl__subcmd__help__subcmd__bar__subcmd__top"
                ;;
            mshellctl__subcmd__help__subcmd__bluetooth,connect)
                cmd="mshellctl__subcmd__help__subcmd__bluetooth__subcmd__connect"
                ;;
            mshellctl__subcmd__help__subcmd__bluetooth,disconnect)
                cmd="mshellctl__subcmd__help__subcmd__bluetooth__subcmd__disconnect"
                ;;
            mshellctl__subcmd__help__subcmd__bluetooth,toggle)
                cmd="mshellctl__subcmd__help__subcmd__bluetooth__subcmd__toggle"
                ;;
            mshellctl__subcmd__help__subcmd__brightness,down)
                cmd="mshellctl__subcmd__help__subcmd__brightness__subcmd__down"
                ;;
            mshellctl__subcmd__help__subcmd__brightness,up)
                cmd="mshellctl__subcmd__help__subcmd__brightness__subcmd__up"
                ;;
            mshellctl__subcmd__help__subcmd__calendar,account)
                cmd="mshellctl__subcmd__help__subcmd__calendar__subcmd__account"
                ;;
            mshellctl__subcmd__help__subcmd__calendar,agenda)
                cmd="mshellctl__subcmd__help__subcmd__calendar__subcmd__agenda"
                ;;
            mshellctl__subcmd__help__subcmd__calendar,on)
                cmd="mshellctl__subcmd__help__subcmd__calendar__subcmd__on"
                ;;
            mshellctl__subcmd__help__subcmd__calendar,today)
                cmd="mshellctl__subcmd__help__subcmd__calendar__subcmd__today"
                ;;
            mshellctl__subcmd__help__subcmd__clipboard,clear)
                cmd="mshellctl__subcmd__help__subcmd__clipboard__subcmd__clear"
                ;;
            mshellctl__subcmd__help__subcmd__clipboard,copy)
                cmd="mshellctl__subcmd__help__subcmd__clipboard__subcmd__copy"
                ;;
            mshellctl__subcmd__help__subcmd__clipboard,delete)
                cmd="mshellctl__subcmd__help__subcmd__clipboard__subcmd__delete"
                ;;
            mshellctl__subcmd__help__subcmd__clipboard,list)
                cmd="mshellctl__subcmd__help__subcmd__clipboard__subcmd__list"
                ;;
            mshellctl__subcmd__help__subcmd__clipboard,pin)
                cmd="mshellctl__subcmd__help__subcmd__clipboard__subcmd__pin"
                ;;
            mshellctl__subcmd__help__subcmd__clipboard,unpin)
                cmd="mshellctl__subcmd__help__subcmd__clipboard__subcmd__unpin"
                ;;
            mshellctl__subcmd__help__subcmd__clipboard,wipe)
                cmd="mshellctl__subcmd__help__subcmd__clipboard__subcmd__wipe"
                ;;
            mshellctl__subcmd__help__subcmd__dock,activate)
                cmd="mshellctl__subcmd__help__subcmd__dock__subcmd__activate"
                ;;
            mshellctl__subcmd__help__subcmd__dock,hide)
                cmd="mshellctl__subcmd__help__subcmd__dock__subcmd__hide"
                ;;
            mshellctl__subcmd__help__subcmd__dock,show)
                cmd="mshellctl__subcmd__help__subcmd__dock__subcmd__show"
                ;;
            mshellctl__subcmd__help__subcmd__dock,toggle)
                cmd="mshellctl__subcmd__help__subcmd__dock__subcmd__toggle"
                ;;
            mshellctl__subcmd__help__subcmd__layout,current)
                cmd="mshellctl__subcmd__help__subcmd__layout__subcmd__current"
                ;;
            mshellctl__subcmd__help__subcmd__layout,list)
                cmd="mshellctl__subcmd__help__subcmd__layout__subcmd__list"
                ;;
            mshellctl__subcmd__help__subcmd__layout,next)
                cmd="mshellctl__subcmd__help__subcmd__layout__subcmd__next"
                ;;
            mshellctl__subcmd__help__subcmd__layout,pick)
                cmd="mshellctl__subcmd__help__subcmd__layout__subcmd__pick"
                ;;
            mshellctl__subcmd__help__subcmd__layout,prev)
                cmd="mshellctl__subcmd__help__subcmd__layout__subcmd__prev"
                ;;
            mshellctl__subcmd__help__subcmd__layout,preview)
                cmd="mshellctl__subcmd__help__subcmd__layout__subcmd__preview"
                ;;
            mshellctl__subcmd__help__subcmd__layout,set)
                cmd="mshellctl__subcmd__help__subcmd__layout__subcmd__set"
                ;;
            mshellctl__subcmd__help__subcmd__lock,activate)
                cmd="mshellctl__subcmd__help__subcmd__lock__subcmd__activate"
                ;;
            mshellctl__subcmd__help__subcmd__lock,check)
                cmd="mshellctl__subcmd__help__subcmd__lock__subcmd__check"
                ;;
            mshellctl__subcmd__help__subcmd__log,disable)
                cmd="mshellctl__subcmd__help__subcmd__log__subcmd__disable"
                ;;
            mshellctl__subcmd__help__subcmd__log,enable)
                cmd="mshellctl__subcmd__help__subcmd__log__subcmd__enable"
                ;;
            mshellctl__subcmd__help__subcmd__log,level)
                cmd="mshellctl__subcmd__help__subcmd__log__subcmd__level"
                ;;
            mshellctl__subcmd__help__subcmd__log,open)
                cmd="mshellctl__subcmd__help__subcmd__log__subcmd__open"
                ;;
            mshellctl__subcmd__help__subcmd__log,path)
                cmd="mshellctl__subcmd__help__subcmd__log__subcmd__path"
                ;;
            mshellctl__subcmd__help__subcmd__media,list)
                cmd="mshellctl__subcmd__help__subcmd__media__subcmd__list"
                ;;
            mshellctl__subcmd__help__subcmd__media,next)
                cmd="mshellctl__subcmd__help__subcmd__media__subcmd__next"
                ;;
            mshellctl__subcmd__help__subcmd__media,prev)
                cmd="mshellctl__subcmd__help__subcmd__media__subcmd__prev"
                ;;
            mshellctl__subcmd__help__subcmd__media,status)
                cmd="mshellctl__subcmd__help__subcmd__media__subcmd__status"
                ;;
            mshellctl__subcmd__help__subcmd__media,toggle)
                cmd="mshellctl__subcmd__help__subcmd__media__subcmd__toggle"
                ;;
            mshellctl__subcmd__help__subcmd__menu,ai)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__ai"
                ;;
            mshellctl__subcmd__help__subcmd__menu,alarm-clock)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__alarm__subcmd__clock"
                ;;
            mshellctl__subcmd__help__subcmd__menu,app-launcher)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__app__subcmd__launcher"
                ;;
            mshellctl__subcmd__help__subcmd__menu,audio-dashboard)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__audio__subcmd__dashboard"
                ;;
            mshellctl__subcmd__help__subcmd__menu,audio-route)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__audio__subcmd__route"
                ;;
            mshellctl__subcmd__help__subcmd__menu,bluetooth)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__bluetooth"
                ;;
            mshellctl__subcmd__help__subcmd__menu,clipboard)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__clipboard"
                ;;
            mshellctl__subcmd__help__subcmd__menu,clock)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__clock"
                ;;
            mshellctl__subcmd__help__subcmd__menu,close-all)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__close__subcmd__all"
                ;;
            mshellctl__subcmd__help__subcmd__menu,control-center)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__control__subcmd__center"
                ;;
            mshellctl__subcmd__help__subcmd__menu,cpu-dashboard)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__cpu__subcmd__dashboard"
                ;;
            mshellctl__subcmd__help__subcmd__menu,dns)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__dns"
                ;;
            mshellctl__subcmd__help__subcmd__menu,ip)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__ip"
                ;;
            mshellctl__subcmd__help__subcmd__menu,keep-awake)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__keep__subcmd__awake"
                ;;
            mshellctl__subcmd__help__subcmd__menu,keybinds)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__keybinds"
                ;;
            mshellctl__subcmd__help__subcmd__menu,lyrics)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__lyrics"
                ;;
            mshellctl__subcmd__help__subcmd__menu,margo-layout)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__margo__subcmd__layout"
                ;;
            mshellctl__subcmd__help__subcmd__menu,mdash)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__mdash"
                ;;
            mshellctl__subcmd__help__subcmd__menu,media-player)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__media__subcmd__player"
                ;;
            mshellctl__subcmd__help__subcmd__menu,network)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__network"
                ;;
            mshellctl__subcmd__help__subcmd__menu,notes)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__notes"
                ;;
            mshellctl__subcmd__help__subcmd__menu,notifications)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__notifications"
                ;;
            mshellctl__subcmd__help__subcmd__menu,plugin)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__plugin"
                ;;
            mshellctl__subcmd__help__subcmd__menu,podman)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__podman"
                ;;
            mshellctl__subcmd__help__subcmd__menu,power)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__power"
                ;;
            mshellctl__subcmd__help__subcmd__menu,privacy)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__privacy"
                ;;
            mshellctl__subcmd__help__subcmd__menu,screenshot)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__screenshot"
                ;;
            mshellctl__subcmd__help__subcmd__menu,session)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__session"
                ;;
            mshellctl__subcmd__help__subcmd__menu,ssh-sessions)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__ssh__subcmd__sessions"
                ;;
            mshellctl__subcmd__help__subcmd__menu,system-update)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__system__subcmd__update"
                ;;
            mshellctl__subcmd__help__subcmd__menu,twilight)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__twilight"
                ;;
            mshellctl__subcmd__help__subcmd__menu,ufw)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__ufw"
                ;;
            mshellctl__subcmd__help__subcmd__menu,valent)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__valent"
                ;;
            mshellctl__subcmd__help__subcmd__menu,vpn)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__vpn"
                ;;
            mshellctl__subcmd__help__subcmd__menu,wallpaper)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__wallpaper"
                ;;
            mshellctl__subcmd__help__subcmd__menu,weather)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__weather"
                ;;
            mshellctl__subcmd__help__subcmd__menu__subcmd__notifications,clears)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__notifications__subcmd__clears"
                ;;
            mshellctl__subcmd__help__subcmd__menu__subcmd__notifications,count)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__notifications__subcmd__count"
                ;;
            mshellctl__subcmd__help__subcmd__menu__subcmd__notifications,dnd)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__notifications__subcmd__dnd"
                ;;
            mshellctl__subcmd__help__subcmd__menu__subcmd__notifications,read)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__notifications__subcmd__read"
                ;;
            mshellctl__subcmd__help__subcmd__menu__subcmd__session,lock)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__session__subcmd__lock"
                ;;
            mshellctl__subcmd__help__subcmd__menu__subcmd__session,logout)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__session__subcmd__logout"
                ;;
            mshellctl__subcmd__help__subcmd__menu__subcmd__session,reboot)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__session__subcmd__reboot"
                ;;
            mshellctl__subcmd__help__subcmd__menu__subcmd__session,shutdown)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__session__subcmd__shutdown"
                ;;
            mshellctl__subcmd__help__subcmd__menu__subcmd__session,suspend)
                cmd="mshellctl__subcmd__help__subcmd__menu__subcmd__session__subcmd__suspend"
                ;;
            mshellctl__subcmd__help__subcmd__notification,clear)
                cmd="mshellctl__subcmd__help__subcmd__notification__subcmd__clear"
                ;;
            mshellctl__subcmd__help__subcmd__notification,count)
                cmd="mshellctl__subcmd__help__subcmd__notification__subcmd__count"
                ;;
            mshellctl__subcmd__help__subcmd__notification,dnd)
                cmd="mshellctl__subcmd__help__subcmd__notification__subcmd__dnd"
                ;;
            mshellctl__subcmd__help__subcmd__notification,open)
                cmd="mshellctl__subcmd__help__subcmd__notification__subcmd__open"
                ;;
            mshellctl__subcmd__help__subcmd__notification,read)
                cmd="mshellctl__subcmd__help__subcmd__notification__subcmd__read"
                ;;
            mshellctl__subcmd__help__subcmd__osk,hide)
                cmd="mshellctl__subcmd__help__subcmd__osk__subcmd__hide"
                ;;
            mshellctl__subcmd__help__subcmd__osk,show)
                cmd="mshellctl__subcmd__help__subcmd__osk__subcmd__show"
                ;;
            mshellctl__subcmd__help__subcmd__osk,toggle)
                cmd="mshellctl__subcmd__help__subcmd__osk__subcmd__toggle"
                ;;
            mshellctl__subcmd__help__subcmd__play,focus)
                cmd="mshellctl__subcmd__help__subcmd__play__subcmd__focus"
                ;;
            mshellctl__subcmd__help__subcmd__play,pin)
                cmd="mshellctl__subcmd__help__subcmd__play__subcmd__pin"
                ;;
            mshellctl__subcmd__help__subcmd__play,play)
                cmd="mshellctl__subcmd__help__subcmd__play__subcmd__play"
                ;;
            mshellctl__subcmd__help__subcmd__play,snap)
                cmd="mshellctl__subcmd__help__subcmd__play__subcmd__snap"
                ;;
            mshellctl__subcmd__help__subcmd__play,start)
                cmd="mshellctl__subcmd__help__subcmd__play__subcmd__start"
                ;;
            mshellctl__subcmd__help__subcmd__play,stop)
                cmd="mshellctl__subcmd__help__subcmd__play__subcmd__stop"
                ;;
            mshellctl__subcmd__help__subcmd__play,toggle)
                cmd="mshellctl__subcmd__help__subcmd__play__subcmd__toggle"
                ;;
            mshellctl__subcmd__help__subcmd__play,wallpaper)
                cmd="mshellctl__subcmd__help__subcmd__play__subcmd__wallpaper"
                ;;
            mshellctl__subcmd__help__subcmd__plugin,keybind)
                cmd="mshellctl__subcmd__help__subcmd__plugin__subcmd__keybind"
                ;;
            mshellctl__subcmd__help__subcmd__plugin,list)
                cmd="mshellctl__subcmd__help__subcmd__plugin__subcmd__list"
                ;;
            mshellctl__subcmd__help__subcmd__plugin,reload)
                cmd="mshellctl__subcmd__help__subcmd__plugin__subcmd__reload"
                ;;
            mshellctl__subcmd__help__subcmd__power,auto)
                cmd="mshellctl__subcmd__help__subcmd__power__subcmd__auto"
                ;;
            mshellctl__subcmd__help__subcmd__power,cycle)
                cmd="mshellctl__subcmd__help__subcmd__power__subcmd__cycle"
                ;;
            mshellctl__subcmd__help__subcmd__power,pause)
                cmd="mshellctl__subcmd__help__subcmd__power__subcmd__pause"
                ;;
            mshellctl__subcmd__help__subcmd__power,resume)
                cmd="mshellctl__subcmd__help__subcmd__power__subcmd__resume"
                ;;
            mshellctl__subcmd__help__subcmd__power,set)
                cmd="mshellctl__subcmd__help__subcmd__power__subcmd__set"
                ;;
            mshellctl__subcmd__help__subcmd__power,status)
                cmd="mshellctl__subcmd__help__subcmd__power__subcmd__status"
                ;;
            mshellctl__subcmd__help__subcmd__screenrecord,start)
                cmd="mshellctl__subcmd__help__subcmd__screenrecord__subcmd__start"
                ;;
            mshellctl__subcmd__help__subcmd__screenrecord,stop)
                cmd="mshellctl__subcmd__help__subcmd__screenrecord__subcmd__stop"
                ;;
            mshellctl__subcmd__help__subcmd__screenrecord,toggle)
                cmd="mshellctl__subcmd__help__subcmd__screenrecord__subcmd__toggle"
                ;;
            mshellctl__subcmd__help__subcmd__screenshot,full)
                cmd="mshellctl__subcmd__help__subcmd__screenshot__subcmd__full"
                ;;
            mshellctl__subcmd__help__subcmd__screenshot,output)
                cmd="mshellctl__subcmd__help__subcmd__screenshot__subcmd__output"
                ;;
            mshellctl__subcmd__help__subcmd__screenshot,region)
                cmd="mshellctl__subcmd__help__subcmd__screenshot__subcmd__region"
                ;;
            mshellctl__subcmd__help__subcmd__screenshot,select-region)
                cmd="mshellctl__subcmd__help__subcmd__screenshot__subcmd__select__subcmd__region"
                ;;
            mshellctl__subcmd__help__subcmd__screenshot,window)
                cmd="mshellctl__subcmd__help__subcmd__screenshot__subcmd__window"
                ;;
            mshellctl__subcmd__help__subcmd__session,lock)
                cmd="mshellctl__subcmd__help__subcmd__session__subcmd__lock"
                ;;
            mshellctl__subcmd__help__subcmd__session,logout)
                cmd="mshellctl__subcmd__help__subcmd__session__subcmd__logout"
                ;;
            mshellctl__subcmd__help__subcmd__session,menu)
                cmd="mshellctl__subcmd__help__subcmd__session__subcmd__menu"
                ;;
            mshellctl__subcmd__help__subcmd__session,reboot)
                cmd="mshellctl__subcmd__help__subcmd__session__subcmd__reboot"
                ;;
            mshellctl__subcmd__help__subcmd__session,shutdown)
                cmd="mshellctl__subcmd__help__subcmd__session__subcmd__shutdown"
                ;;
            mshellctl__subcmd__help__subcmd__session,suspend)
                cmd="mshellctl__subcmd__help__subcmd__session__subcmd__suspend"
                ;;
            mshellctl__subcmd__help__subcmd__settings,close)
                cmd="mshellctl__subcmd__help__subcmd__settings__subcmd__close"
                ;;
            mshellctl__subcmd__help__subcmd__settings,open)
                cmd="mshellctl__subcmd__help__subcmd__settings__subcmd__open"
                ;;
            mshellctl__subcmd__help__subcmd__theme,get)
                cmd="mshellctl__subcmd__help__subcmd__theme__subcmd__get"
                ;;
            mshellctl__subcmd__help__subcmd__theme,list)
                cmd="mshellctl__subcmd__help__subcmd__theme__subcmd__list"
                ;;
            mshellctl__subcmd__help__subcmd__theme,set)
                cmd="mshellctl__subcmd__help__subcmd__theme__subcmd__set"
                ;;
            mshellctl__subcmd__help__subcmd__vpn,connect)
                cmd="mshellctl__subcmd__help__subcmd__vpn__subcmd__connect"
                ;;
            mshellctl__subcmd__help__subcmd__vpn,disconnect)
                cmd="mshellctl__subcmd__help__subcmd__vpn__subcmd__disconnect"
                ;;
            mshellctl__subcmd__help__subcmd__vpn,fastest)
                cmd="mshellctl__subcmd__help__subcmd__vpn__subcmd__fastest"
                ;;
            mshellctl__subcmd__help__subcmd__vpn,menu)
                cmd="mshellctl__subcmd__help__subcmd__vpn__subcmd__menu"
                ;;
            mshellctl__subcmd__help__subcmd__vpn,random)
                cmd="mshellctl__subcmd__help__subcmd__vpn__subcmd__random"
                ;;
            mshellctl__subcmd__help__subcmd__vpn,reconnect)
                cmd="mshellctl__subcmd__help__subcmd__vpn__subcmd__reconnect"
                ;;
            mshellctl__subcmd__help__subcmd__vpn,status)
                cmd="mshellctl__subcmd__help__subcmd__vpn__subcmd__status"
                ;;
            mshellctl__subcmd__help__subcmd__vpn,toggle)
                cmd="mshellctl__subcmd__help__subcmd__vpn__subcmd__toggle"
                ;;
            mshellctl__subcmd__help__subcmd__wallpaper,next)
                cmd="mshellctl__subcmd__help__subcmd__wallpaper__subcmd__next"
                ;;
            mshellctl__subcmd__help__subcmd__wallpaper,prev)
                cmd="mshellctl__subcmd__help__subcmd__wallpaper__subcmd__prev"
                ;;
            mshellctl__subcmd__help__subcmd__wallpaper,random)
                cmd="mshellctl__subcmd__help__subcmd__wallpaper__subcmd__random"
                ;;
            mshellctl__subcmd__layout,current)
                cmd="mshellctl__subcmd__layout__subcmd__current"
                ;;
            mshellctl__subcmd__layout,help)
                cmd="mshellctl__subcmd__layout__subcmd__help"
                ;;
            mshellctl__subcmd__layout,list)
                cmd="mshellctl__subcmd__layout__subcmd__list"
                ;;
            mshellctl__subcmd__layout,next)
                cmd="mshellctl__subcmd__layout__subcmd__next"
                ;;
            mshellctl__subcmd__layout,pick)
                cmd="mshellctl__subcmd__layout__subcmd__pick"
                ;;
            mshellctl__subcmd__layout,prev)
                cmd="mshellctl__subcmd__layout__subcmd__prev"
                ;;
            mshellctl__subcmd__layout,preview)
                cmd="mshellctl__subcmd__layout__subcmd__preview"
                ;;
            mshellctl__subcmd__layout,set)
                cmd="mshellctl__subcmd__layout__subcmd__set"
                ;;
            mshellctl__subcmd__layout__subcmd__help,current)
                cmd="mshellctl__subcmd__layout__subcmd__help__subcmd__current"
                ;;
            mshellctl__subcmd__layout__subcmd__help,help)
                cmd="mshellctl__subcmd__layout__subcmd__help__subcmd__help"
                ;;
            mshellctl__subcmd__layout__subcmd__help,list)
                cmd="mshellctl__subcmd__layout__subcmd__help__subcmd__list"
                ;;
            mshellctl__subcmd__layout__subcmd__help,next)
                cmd="mshellctl__subcmd__layout__subcmd__help__subcmd__next"
                ;;
            mshellctl__subcmd__layout__subcmd__help,pick)
                cmd="mshellctl__subcmd__layout__subcmd__help__subcmd__pick"
                ;;
            mshellctl__subcmd__layout__subcmd__help,prev)
                cmd="mshellctl__subcmd__layout__subcmd__help__subcmd__prev"
                ;;
            mshellctl__subcmd__layout__subcmd__help,preview)
                cmd="mshellctl__subcmd__layout__subcmd__help__subcmd__preview"
                ;;
            mshellctl__subcmd__layout__subcmd__help,set)
                cmd="mshellctl__subcmd__layout__subcmd__help__subcmd__set"
                ;;
            mshellctl__subcmd__lock,activate)
                cmd="mshellctl__subcmd__lock__subcmd__activate"
                ;;
            mshellctl__subcmd__lock,check)
                cmd="mshellctl__subcmd__lock__subcmd__check"
                ;;
            mshellctl__subcmd__lock,help)
                cmd="mshellctl__subcmd__lock__subcmd__help"
                ;;
            mshellctl__subcmd__lock__subcmd__help,activate)
                cmd="mshellctl__subcmd__lock__subcmd__help__subcmd__activate"
                ;;
            mshellctl__subcmd__lock__subcmd__help,check)
                cmd="mshellctl__subcmd__lock__subcmd__help__subcmd__check"
                ;;
            mshellctl__subcmd__lock__subcmd__help,help)
                cmd="mshellctl__subcmd__lock__subcmd__help__subcmd__help"
                ;;
            mshellctl__subcmd__log,disable)
                cmd="mshellctl__subcmd__log__subcmd__disable"
                ;;
            mshellctl__subcmd__log,enable)
                cmd="mshellctl__subcmd__log__subcmd__enable"
                ;;
            mshellctl__subcmd__log,help)
                cmd="mshellctl__subcmd__log__subcmd__help"
                ;;
            mshellctl__subcmd__log,level)
                cmd="mshellctl__subcmd__log__subcmd__level"
                ;;
            mshellctl__subcmd__log,open)
                cmd="mshellctl__subcmd__log__subcmd__open"
                ;;
            mshellctl__subcmd__log,path)
                cmd="mshellctl__subcmd__log__subcmd__path"
                ;;
            mshellctl__subcmd__log__subcmd__help,disable)
                cmd="mshellctl__subcmd__log__subcmd__help__subcmd__disable"
                ;;
            mshellctl__subcmd__log__subcmd__help,enable)
                cmd="mshellctl__subcmd__log__subcmd__help__subcmd__enable"
                ;;
            mshellctl__subcmd__log__subcmd__help,help)
                cmd="mshellctl__subcmd__log__subcmd__help__subcmd__help"
                ;;
            mshellctl__subcmd__log__subcmd__help,level)
                cmd="mshellctl__subcmd__log__subcmd__help__subcmd__level"
                ;;
            mshellctl__subcmd__log__subcmd__help,open)
                cmd="mshellctl__subcmd__log__subcmd__help__subcmd__open"
                ;;
            mshellctl__subcmd__log__subcmd__help,path)
                cmd="mshellctl__subcmd__log__subcmd__help__subcmd__path"
                ;;
            mshellctl__subcmd__media,help)
                cmd="mshellctl__subcmd__media__subcmd__help"
                ;;
            mshellctl__subcmd__media,list)
                cmd="mshellctl__subcmd__media__subcmd__list"
                ;;
            mshellctl__subcmd__media,next)
                cmd="mshellctl__subcmd__media__subcmd__next"
                ;;
            mshellctl__subcmd__media,prev)
                cmd="mshellctl__subcmd__media__subcmd__prev"
                ;;
            mshellctl__subcmd__media,status)
                cmd="mshellctl__subcmd__media__subcmd__status"
                ;;
            mshellctl__subcmd__media,toggle)
                cmd="mshellctl__subcmd__media__subcmd__toggle"
                ;;
            mshellctl__subcmd__media__subcmd__help,help)
                cmd="mshellctl__subcmd__media__subcmd__help__subcmd__help"
                ;;
            mshellctl__subcmd__media__subcmd__help,list)
                cmd="mshellctl__subcmd__media__subcmd__help__subcmd__list"
                ;;
            mshellctl__subcmd__media__subcmd__help,next)
                cmd="mshellctl__subcmd__media__subcmd__help__subcmd__next"
                ;;
            mshellctl__subcmd__media__subcmd__help,prev)
                cmd="mshellctl__subcmd__media__subcmd__help__subcmd__prev"
                ;;
            mshellctl__subcmd__media__subcmd__help,status)
                cmd="mshellctl__subcmd__media__subcmd__help__subcmd__status"
                ;;
            mshellctl__subcmd__media__subcmd__help,toggle)
                cmd="mshellctl__subcmd__media__subcmd__help__subcmd__toggle"
                ;;
            mshellctl__subcmd__menu,ai)
                cmd="mshellctl__subcmd__menu__subcmd__ai"
                ;;
            mshellctl__subcmd__menu,alarm-clock)
                cmd="mshellctl__subcmd__menu__subcmd__alarm__subcmd__clock"
                ;;
            mshellctl__subcmd__menu,app-launcher)
                cmd="mshellctl__subcmd__menu__subcmd__app__subcmd__launcher"
                ;;
            mshellctl__subcmd__menu,audio-dashboard)
                cmd="mshellctl__subcmd__menu__subcmd__audio__subcmd__dashboard"
                ;;
            mshellctl__subcmd__menu,audio-route)
                cmd="mshellctl__subcmd__menu__subcmd__audio__subcmd__route"
                ;;
            mshellctl__subcmd__menu,bluetooth)
                cmd="mshellctl__subcmd__menu__subcmd__bluetooth"
                ;;
            mshellctl__subcmd__menu,clipboard)
                cmd="mshellctl__subcmd__menu__subcmd__clipboard"
                ;;
            mshellctl__subcmd__menu,clock)
                cmd="mshellctl__subcmd__menu__subcmd__clock"
                ;;
            mshellctl__subcmd__menu,close-all)
                cmd="mshellctl__subcmd__menu__subcmd__close__subcmd__all"
                ;;
            mshellctl__subcmd__menu,control-center)
                cmd="mshellctl__subcmd__menu__subcmd__control__subcmd__center"
                ;;
            mshellctl__subcmd__menu,cpu-dashboard)
                cmd="mshellctl__subcmd__menu__subcmd__cpu__subcmd__dashboard"
                ;;
            mshellctl__subcmd__menu,dns)
                cmd="mshellctl__subcmd__menu__subcmd__dns"
                ;;
            mshellctl__subcmd__menu,help)
                cmd="mshellctl__subcmd__menu__subcmd__help"
                ;;
            mshellctl__subcmd__menu,ip)
                cmd="mshellctl__subcmd__menu__subcmd__ip"
                ;;
            mshellctl__subcmd__menu,keep-awake)
                cmd="mshellctl__subcmd__menu__subcmd__keep__subcmd__awake"
                ;;
            mshellctl__subcmd__menu,keybinds)
                cmd="mshellctl__subcmd__menu__subcmd__keybinds"
                ;;
            mshellctl__subcmd__menu,lyrics)
                cmd="mshellctl__subcmd__menu__subcmd__lyrics"
                ;;
            mshellctl__subcmd__menu,margo-layout)
                cmd="mshellctl__subcmd__menu__subcmd__margo__subcmd__layout"
                ;;
            mshellctl__subcmd__menu,mdash)
                cmd="mshellctl__subcmd__menu__subcmd__mdash"
                ;;
            mshellctl__subcmd__menu,media-player)
                cmd="mshellctl__subcmd__menu__subcmd__media__subcmd__player"
                ;;
            mshellctl__subcmd__menu,network)
                cmd="mshellctl__subcmd__menu__subcmd__network"
                ;;
            mshellctl__subcmd__menu,notes)
                cmd="mshellctl__subcmd__menu__subcmd__notes"
                ;;
            mshellctl__subcmd__menu,notifications)
                cmd="mshellctl__subcmd__menu__subcmd__notifications"
                ;;
            mshellctl__subcmd__menu,plugin)
                cmd="mshellctl__subcmd__menu__subcmd__plugin"
                ;;
            mshellctl__subcmd__menu,podman)
                cmd="mshellctl__subcmd__menu__subcmd__podman"
                ;;
            mshellctl__subcmd__menu,power)
                cmd="mshellctl__subcmd__menu__subcmd__power"
                ;;
            mshellctl__subcmd__menu,privacy)
                cmd="mshellctl__subcmd__menu__subcmd__privacy"
                ;;
            mshellctl__subcmd__menu,screenshot)
                cmd="mshellctl__subcmd__menu__subcmd__screenshot"
                ;;
            mshellctl__subcmd__menu,session)
                cmd="mshellctl__subcmd__menu__subcmd__session"
                ;;
            mshellctl__subcmd__menu,ssh-sessions)
                cmd="mshellctl__subcmd__menu__subcmd__ssh__subcmd__sessions"
                ;;
            mshellctl__subcmd__menu,system-update)
                cmd="mshellctl__subcmd__menu__subcmd__system__subcmd__update"
                ;;
            mshellctl__subcmd__menu,twilight)
                cmd="mshellctl__subcmd__menu__subcmd__twilight"
                ;;
            mshellctl__subcmd__menu,ufw)
                cmd="mshellctl__subcmd__menu__subcmd__ufw"
                ;;
            mshellctl__subcmd__menu,valent)
                cmd="mshellctl__subcmd__menu__subcmd__valent"
                ;;
            mshellctl__subcmd__menu,vpn)
                cmd="mshellctl__subcmd__menu__subcmd__vpn"
                ;;
            mshellctl__subcmd__menu,wallpaper)
                cmd="mshellctl__subcmd__menu__subcmd__wallpaper"
                ;;
            mshellctl__subcmd__menu,weather)
                cmd="mshellctl__subcmd__menu__subcmd__weather"
                ;;
            mshellctl__subcmd__menu__subcmd__help,ai)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__ai"
                ;;
            mshellctl__subcmd__menu__subcmd__help,alarm-clock)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__alarm__subcmd__clock"
                ;;
            mshellctl__subcmd__menu__subcmd__help,app-launcher)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__app__subcmd__launcher"
                ;;
            mshellctl__subcmd__menu__subcmd__help,audio-dashboard)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__audio__subcmd__dashboard"
                ;;
            mshellctl__subcmd__menu__subcmd__help,audio-route)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__audio__subcmd__route"
                ;;
            mshellctl__subcmd__menu__subcmd__help,bluetooth)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__bluetooth"
                ;;
            mshellctl__subcmd__menu__subcmd__help,clipboard)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__clipboard"
                ;;
            mshellctl__subcmd__menu__subcmd__help,clock)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__clock"
                ;;
            mshellctl__subcmd__menu__subcmd__help,close-all)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__close__subcmd__all"
                ;;
            mshellctl__subcmd__menu__subcmd__help,control-center)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__control__subcmd__center"
                ;;
            mshellctl__subcmd__menu__subcmd__help,cpu-dashboard)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__cpu__subcmd__dashboard"
                ;;
            mshellctl__subcmd__menu__subcmd__help,dns)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__dns"
                ;;
            mshellctl__subcmd__menu__subcmd__help,help)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__help"
                ;;
            mshellctl__subcmd__menu__subcmd__help,ip)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__ip"
                ;;
            mshellctl__subcmd__menu__subcmd__help,keep-awake)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__keep__subcmd__awake"
                ;;
            mshellctl__subcmd__menu__subcmd__help,keybinds)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__keybinds"
                ;;
            mshellctl__subcmd__menu__subcmd__help,lyrics)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__lyrics"
                ;;
            mshellctl__subcmd__menu__subcmd__help,margo-layout)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__margo__subcmd__layout"
                ;;
            mshellctl__subcmd__menu__subcmd__help,mdash)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__mdash"
                ;;
            mshellctl__subcmd__menu__subcmd__help,media-player)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__media__subcmd__player"
                ;;
            mshellctl__subcmd__menu__subcmd__help,network)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__network"
                ;;
            mshellctl__subcmd__menu__subcmd__help,notes)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__notes"
                ;;
            mshellctl__subcmd__menu__subcmd__help,notifications)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__notifications"
                ;;
            mshellctl__subcmd__menu__subcmd__help,plugin)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__plugin"
                ;;
            mshellctl__subcmd__menu__subcmd__help,podman)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__podman"
                ;;
            mshellctl__subcmd__menu__subcmd__help,power)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__power"
                ;;
            mshellctl__subcmd__menu__subcmd__help,privacy)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__privacy"
                ;;
            mshellctl__subcmd__menu__subcmd__help,screenshot)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__screenshot"
                ;;
            mshellctl__subcmd__menu__subcmd__help,session)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__session"
                ;;
            mshellctl__subcmd__menu__subcmd__help,ssh-sessions)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__ssh__subcmd__sessions"
                ;;
            mshellctl__subcmd__menu__subcmd__help,system-update)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__system__subcmd__update"
                ;;
            mshellctl__subcmd__menu__subcmd__help,twilight)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__twilight"
                ;;
            mshellctl__subcmd__menu__subcmd__help,ufw)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__ufw"
                ;;
            mshellctl__subcmd__menu__subcmd__help,valent)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__valent"
                ;;
            mshellctl__subcmd__menu__subcmd__help,vpn)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__vpn"
                ;;
            mshellctl__subcmd__menu__subcmd__help,wallpaper)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__wallpaper"
                ;;
            mshellctl__subcmd__menu__subcmd__help,weather)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__weather"
                ;;
            mshellctl__subcmd__menu__subcmd__help__subcmd__notifications,clears)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__notifications__subcmd__clears"
                ;;
            mshellctl__subcmd__menu__subcmd__help__subcmd__notifications,count)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__notifications__subcmd__count"
                ;;
            mshellctl__subcmd__menu__subcmd__help__subcmd__notifications,dnd)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__notifications__subcmd__dnd"
                ;;
            mshellctl__subcmd__menu__subcmd__help__subcmd__notifications,read)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__notifications__subcmd__read"
                ;;
            mshellctl__subcmd__menu__subcmd__help__subcmd__session,lock)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__session__subcmd__lock"
                ;;
            mshellctl__subcmd__menu__subcmd__help__subcmd__session,logout)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__session__subcmd__logout"
                ;;
            mshellctl__subcmd__menu__subcmd__help__subcmd__session,reboot)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__session__subcmd__reboot"
                ;;
            mshellctl__subcmd__menu__subcmd__help__subcmd__session,shutdown)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__session__subcmd__shutdown"
                ;;
            mshellctl__subcmd__menu__subcmd__help__subcmd__session,suspend)
                cmd="mshellctl__subcmd__menu__subcmd__help__subcmd__session__subcmd__suspend"
                ;;
            mshellctl__subcmd__menu__subcmd__notifications,clears)
                cmd="mshellctl__subcmd__menu__subcmd__notifications__subcmd__clears"
                ;;
            mshellctl__subcmd__menu__subcmd__notifications,count)
                cmd="mshellctl__subcmd__menu__subcmd__notifications__subcmd__count"
                ;;
            mshellctl__subcmd__menu__subcmd__notifications,dnd)
                cmd="mshellctl__subcmd__menu__subcmd__notifications__subcmd__dnd"
                ;;
            mshellctl__subcmd__menu__subcmd__notifications,help)
                cmd="mshellctl__subcmd__menu__subcmd__notifications__subcmd__help"
                ;;
            mshellctl__subcmd__menu__subcmd__notifications,read)
                cmd="mshellctl__subcmd__menu__subcmd__notifications__subcmd__read"
                ;;
            mshellctl__subcmd__menu__subcmd__notifications__subcmd__help,clears)
                cmd="mshellctl__subcmd__menu__subcmd__notifications__subcmd__help__subcmd__clears"
                ;;
            mshellctl__subcmd__menu__subcmd__notifications__subcmd__help,count)
                cmd="mshellctl__subcmd__menu__subcmd__notifications__subcmd__help__subcmd__count"
                ;;
            mshellctl__subcmd__menu__subcmd__notifications__subcmd__help,dnd)
                cmd="mshellctl__subcmd__menu__subcmd__notifications__subcmd__help__subcmd__dnd"
                ;;
            mshellctl__subcmd__menu__subcmd__notifications__subcmd__help,help)
                cmd="mshellctl__subcmd__menu__subcmd__notifications__subcmd__help__subcmd__help"
                ;;
            mshellctl__subcmd__menu__subcmd__notifications__subcmd__help,read)
                cmd="mshellctl__subcmd__menu__subcmd__notifications__subcmd__help__subcmd__read"
                ;;
            mshellctl__subcmd__menu__subcmd__session,help)
                cmd="mshellctl__subcmd__menu__subcmd__session__subcmd__help"
                ;;
            mshellctl__subcmd__menu__subcmd__session,lock)
                cmd="mshellctl__subcmd__menu__subcmd__session__subcmd__lock"
                ;;
            mshellctl__subcmd__menu__subcmd__session,logout)
                cmd="mshellctl__subcmd__menu__subcmd__session__subcmd__logout"
                ;;
            mshellctl__subcmd__menu__subcmd__session,reboot)
                cmd="mshellctl__subcmd__menu__subcmd__session__subcmd__reboot"
                ;;
            mshellctl__subcmd__menu__subcmd__session,shutdown)
                cmd="mshellctl__subcmd__menu__subcmd__session__subcmd__shutdown"
                ;;
            mshellctl__subcmd__menu__subcmd__session,suspend)
                cmd="mshellctl__subcmd__menu__subcmd__session__subcmd__suspend"
                ;;
            mshellctl__subcmd__menu__subcmd__session__subcmd__help,help)
                cmd="mshellctl__subcmd__menu__subcmd__session__subcmd__help__subcmd__help"
                ;;
            mshellctl__subcmd__menu__subcmd__session__subcmd__help,lock)
                cmd="mshellctl__subcmd__menu__subcmd__session__subcmd__help__subcmd__lock"
                ;;
            mshellctl__subcmd__menu__subcmd__session__subcmd__help,logout)
                cmd="mshellctl__subcmd__menu__subcmd__session__subcmd__help__subcmd__logout"
                ;;
            mshellctl__subcmd__menu__subcmd__session__subcmd__help,reboot)
                cmd="mshellctl__subcmd__menu__subcmd__session__subcmd__help__subcmd__reboot"
                ;;
            mshellctl__subcmd__menu__subcmd__session__subcmd__help,shutdown)
                cmd="mshellctl__subcmd__menu__subcmd__session__subcmd__help__subcmd__shutdown"
                ;;
            mshellctl__subcmd__menu__subcmd__session__subcmd__help,suspend)
                cmd="mshellctl__subcmd__menu__subcmd__session__subcmd__help__subcmd__suspend"
                ;;
            mshellctl__subcmd__notification,clear)
                cmd="mshellctl__subcmd__notification__subcmd__clear"
                ;;
            mshellctl__subcmd__notification,count)
                cmd="mshellctl__subcmd__notification__subcmd__count"
                ;;
            mshellctl__subcmd__notification,dnd)
                cmd="mshellctl__subcmd__notification__subcmd__dnd"
                ;;
            mshellctl__subcmd__notification,help)
                cmd="mshellctl__subcmd__notification__subcmd__help"
                ;;
            mshellctl__subcmd__notification,open)
                cmd="mshellctl__subcmd__notification__subcmd__open"
                ;;
            mshellctl__subcmd__notification,read)
                cmd="mshellctl__subcmd__notification__subcmd__read"
                ;;
            mshellctl__subcmd__notification__subcmd__help,clear)
                cmd="mshellctl__subcmd__notification__subcmd__help__subcmd__clear"
                ;;
            mshellctl__subcmd__notification__subcmd__help,count)
                cmd="mshellctl__subcmd__notification__subcmd__help__subcmd__count"
                ;;
            mshellctl__subcmd__notification__subcmd__help,dnd)
                cmd="mshellctl__subcmd__notification__subcmd__help__subcmd__dnd"
                ;;
            mshellctl__subcmd__notification__subcmd__help,help)
                cmd="mshellctl__subcmd__notification__subcmd__help__subcmd__help"
                ;;
            mshellctl__subcmd__notification__subcmd__help,open)
                cmd="mshellctl__subcmd__notification__subcmd__help__subcmd__open"
                ;;
            mshellctl__subcmd__notification__subcmd__help,read)
                cmd="mshellctl__subcmd__notification__subcmd__help__subcmd__read"
                ;;
            mshellctl__subcmd__osk,help)
                cmd="mshellctl__subcmd__osk__subcmd__help"
                ;;
            mshellctl__subcmd__osk,hide)
                cmd="mshellctl__subcmd__osk__subcmd__hide"
                ;;
            mshellctl__subcmd__osk,show)
                cmd="mshellctl__subcmd__osk__subcmd__show"
                ;;
            mshellctl__subcmd__osk,toggle)
                cmd="mshellctl__subcmd__osk__subcmd__toggle"
                ;;
            mshellctl__subcmd__osk__subcmd__help,help)
                cmd="mshellctl__subcmd__osk__subcmd__help__subcmd__help"
                ;;
            mshellctl__subcmd__osk__subcmd__help,hide)
                cmd="mshellctl__subcmd__osk__subcmd__help__subcmd__hide"
                ;;
            mshellctl__subcmd__osk__subcmd__help,show)
                cmd="mshellctl__subcmd__osk__subcmd__help__subcmd__show"
                ;;
            mshellctl__subcmd__osk__subcmd__help,toggle)
                cmd="mshellctl__subcmd__osk__subcmd__help__subcmd__toggle"
                ;;
            mshellctl__subcmd__play,focus)
                cmd="mshellctl__subcmd__play__subcmd__focus"
                ;;
            mshellctl__subcmd__play,help)
                cmd="mshellctl__subcmd__play__subcmd__help"
                ;;
            mshellctl__subcmd__play,pin)
                cmd="mshellctl__subcmd__play__subcmd__pin"
                ;;
            mshellctl__subcmd__play,play)
                cmd="mshellctl__subcmd__play__subcmd__play"
                ;;
            mshellctl__subcmd__play,snap)
                cmd="mshellctl__subcmd__play__subcmd__snap"
                ;;
            mshellctl__subcmd__play,start)
                cmd="mshellctl__subcmd__play__subcmd__start"
                ;;
            mshellctl__subcmd__play,stop)
                cmd="mshellctl__subcmd__play__subcmd__stop"
                ;;
            mshellctl__subcmd__play,toggle)
                cmd="mshellctl__subcmd__play__subcmd__toggle"
                ;;
            mshellctl__subcmd__play,wallpaper)
                cmd="mshellctl__subcmd__play__subcmd__wallpaper"
                ;;
            mshellctl__subcmd__play__subcmd__help,focus)
                cmd="mshellctl__subcmd__play__subcmd__help__subcmd__focus"
                ;;
            mshellctl__subcmd__play__subcmd__help,help)
                cmd="mshellctl__subcmd__play__subcmd__help__subcmd__help"
                ;;
            mshellctl__subcmd__play__subcmd__help,pin)
                cmd="mshellctl__subcmd__play__subcmd__help__subcmd__pin"
                ;;
            mshellctl__subcmd__play__subcmd__help,play)
                cmd="mshellctl__subcmd__play__subcmd__help__subcmd__play"
                ;;
            mshellctl__subcmd__play__subcmd__help,snap)
                cmd="mshellctl__subcmd__play__subcmd__help__subcmd__snap"
                ;;
            mshellctl__subcmd__play__subcmd__help,start)
                cmd="mshellctl__subcmd__play__subcmd__help__subcmd__start"
                ;;
            mshellctl__subcmd__play__subcmd__help,stop)
                cmd="mshellctl__subcmd__play__subcmd__help__subcmd__stop"
                ;;
            mshellctl__subcmd__play__subcmd__help,toggle)
                cmd="mshellctl__subcmd__play__subcmd__help__subcmd__toggle"
                ;;
            mshellctl__subcmd__play__subcmd__help,wallpaper)
                cmd="mshellctl__subcmd__play__subcmd__help__subcmd__wallpaper"
                ;;
            mshellctl__subcmd__plugin,help)
                cmd="mshellctl__subcmd__plugin__subcmd__help"
                ;;
            mshellctl__subcmd__plugin,keybind)
                cmd="mshellctl__subcmd__plugin__subcmd__keybind"
                ;;
            mshellctl__subcmd__plugin,list)
                cmd="mshellctl__subcmd__plugin__subcmd__list"
                ;;
            mshellctl__subcmd__plugin,reload)
                cmd="mshellctl__subcmd__plugin__subcmd__reload"
                ;;
            mshellctl__subcmd__plugin__subcmd__help,help)
                cmd="mshellctl__subcmd__plugin__subcmd__help__subcmd__help"
                ;;
            mshellctl__subcmd__plugin__subcmd__help,keybind)
                cmd="mshellctl__subcmd__plugin__subcmd__help__subcmd__keybind"
                ;;
            mshellctl__subcmd__plugin__subcmd__help,list)
                cmd="mshellctl__subcmd__plugin__subcmd__help__subcmd__list"
                ;;
            mshellctl__subcmd__plugin__subcmd__help,reload)
                cmd="mshellctl__subcmd__plugin__subcmd__help__subcmd__reload"
                ;;
            mshellctl__subcmd__power,auto)
                cmd="mshellctl__subcmd__power__subcmd__auto"
                ;;
            mshellctl__subcmd__power,cycle)
                cmd="mshellctl__subcmd__power__subcmd__cycle"
                ;;
            mshellctl__subcmd__power,help)
                cmd="mshellctl__subcmd__power__subcmd__help"
                ;;
            mshellctl__subcmd__power,pause)
                cmd="mshellctl__subcmd__power__subcmd__pause"
                ;;
            mshellctl__subcmd__power,resume)
                cmd="mshellctl__subcmd__power__subcmd__resume"
                ;;
            mshellctl__subcmd__power,set)
                cmd="mshellctl__subcmd__power__subcmd__set"
                ;;
            mshellctl__subcmd__power,status)
                cmd="mshellctl__subcmd__power__subcmd__status"
                ;;
            mshellctl__subcmd__power__subcmd__help,auto)
                cmd="mshellctl__subcmd__power__subcmd__help__subcmd__auto"
                ;;
            mshellctl__subcmd__power__subcmd__help,cycle)
                cmd="mshellctl__subcmd__power__subcmd__help__subcmd__cycle"
                ;;
            mshellctl__subcmd__power__subcmd__help,help)
                cmd="mshellctl__subcmd__power__subcmd__help__subcmd__help"
                ;;
            mshellctl__subcmd__power__subcmd__help,pause)
                cmd="mshellctl__subcmd__power__subcmd__help__subcmd__pause"
                ;;
            mshellctl__subcmd__power__subcmd__help,resume)
                cmd="mshellctl__subcmd__power__subcmd__help__subcmd__resume"
                ;;
            mshellctl__subcmd__power__subcmd__help,set)
                cmd="mshellctl__subcmd__power__subcmd__help__subcmd__set"
                ;;
            mshellctl__subcmd__power__subcmd__help,status)
                cmd="mshellctl__subcmd__power__subcmd__help__subcmd__status"
                ;;
            mshellctl__subcmd__screenrecord,help)
                cmd="mshellctl__subcmd__screenrecord__subcmd__help"
                ;;
            mshellctl__subcmd__screenrecord,start)
                cmd="mshellctl__subcmd__screenrecord__subcmd__start"
                ;;
            mshellctl__subcmd__screenrecord,stop)
                cmd="mshellctl__subcmd__screenrecord__subcmd__stop"
                ;;
            mshellctl__subcmd__screenrecord,toggle)
                cmd="mshellctl__subcmd__screenrecord__subcmd__toggle"
                ;;
            mshellctl__subcmd__screenrecord__subcmd__help,help)
                cmd="mshellctl__subcmd__screenrecord__subcmd__help__subcmd__help"
                ;;
            mshellctl__subcmd__screenrecord__subcmd__help,start)
                cmd="mshellctl__subcmd__screenrecord__subcmd__help__subcmd__start"
                ;;
            mshellctl__subcmd__screenrecord__subcmd__help,stop)
                cmd="mshellctl__subcmd__screenrecord__subcmd__help__subcmd__stop"
                ;;
            mshellctl__subcmd__screenrecord__subcmd__help,toggle)
                cmd="mshellctl__subcmd__screenrecord__subcmd__help__subcmd__toggle"
                ;;
            mshellctl__subcmd__screenshot,full)
                cmd="mshellctl__subcmd__screenshot__subcmd__full"
                ;;
            mshellctl__subcmd__screenshot,help)
                cmd="mshellctl__subcmd__screenshot__subcmd__help"
                ;;
            mshellctl__subcmd__screenshot,output)
                cmd="mshellctl__subcmd__screenshot__subcmd__output"
                ;;
            mshellctl__subcmd__screenshot,region)
                cmd="mshellctl__subcmd__screenshot__subcmd__region"
                ;;
            mshellctl__subcmd__screenshot,select-region)
                cmd="mshellctl__subcmd__screenshot__subcmd__select__subcmd__region"
                ;;
            mshellctl__subcmd__screenshot,window)
                cmd="mshellctl__subcmd__screenshot__subcmd__window"
                ;;
            mshellctl__subcmd__screenshot__subcmd__help,full)
                cmd="mshellctl__subcmd__screenshot__subcmd__help__subcmd__full"
                ;;
            mshellctl__subcmd__screenshot__subcmd__help,help)
                cmd="mshellctl__subcmd__screenshot__subcmd__help__subcmd__help"
                ;;
            mshellctl__subcmd__screenshot__subcmd__help,output)
                cmd="mshellctl__subcmd__screenshot__subcmd__help__subcmd__output"
                ;;
            mshellctl__subcmd__screenshot__subcmd__help,region)
                cmd="mshellctl__subcmd__screenshot__subcmd__help__subcmd__region"
                ;;
            mshellctl__subcmd__screenshot__subcmd__help,select-region)
                cmd="mshellctl__subcmd__screenshot__subcmd__help__subcmd__select__subcmd__region"
                ;;
            mshellctl__subcmd__screenshot__subcmd__help,window)
                cmd="mshellctl__subcmd__screenshot__subcmd__help__subcmd__window"
                ;;
            mshellctl__subcmd__session,help)
                cmd="mshellctl__subcmd__session__subcmd__help"
                ;;
            mshellctl__subcmd__session,lock)
                cmd="mshellctl__subcmd__session__subcmd__lock"
                ;;
            mshellctl__subcmd__session,logout)
                cmd="mshellctl__subcmd__session__subcmd__logout"
                ;;
            mshellctl__subcmd__session,menu)
                cmd="mshellctl__subcmd__session__subcmd__menu"
                ;;
            mshellctl__subcmd__session,reboot)
                cmd="mshellctl__subcmd__session__subcmd__reboot"
                ;;
            mshellctl__subcmd__session,shutdown)
                cmd="mshellctl__subcmd__session__subcmd__shutdown"
                ;;
            mshellctl__subcmd__session,suspend)
                cmd="mshellctl__subcmd__session__subcmd__suspend"
                ;;
            mshellctl__subcmd__session__subcmd__help,help)
                cmd="mshellctl__subcmd__session__subcmd__help__subcmd__help"
                ;;
            mshellctl__subcmd__session__subcmd__help,lock)
                cmd="mshellctl__subcmd__session__subcmd__help__subcmd__lock"
                ;;
            mshellctl__subcmd__session__subcmd__help,logout)
                cmd="mshellctl__subcmd__session__subcmd__help__subcmd__logout"
                ;;
            mshellctl__subcmd__session__subcmd__help,menu)
                cmd="mshellctl__subcmd__session__subcmd__help__subcmd__menu"
                ;;
            mshellctl__subcmd__session__subcmd__help,reboot)
                cmd="mshellctl__subcmd__session__subcmd__help__subcmd__reboot"
                ;;
            mshellctl__subcmd__session__subcmd__help,shutdown)
                cmd="mshellctl__subcmd__session__subcmd__help__subcmd__shutdown"
                ;;
            mshellctl__subcmd__session__subcmd__help,suspend)
                cmd="mshellctl__subcmd__session__subcmd__help__subcmd__suspend"
                ;;
            mshellctl__subcmd__settings,close)
                cmd="mshellctl__subcmd__settings__subcmd__close"
                ;;
            mshellctl__subcmd__settings,help)
                cmd="mshellctl__subcmd__settings__subcmd__help"
                ;;
            mshellctl__subcmd__settings,open)
                cmd="mshellctl__subcmd__settings__subcmd__open"
                ;;
            mshellctl__subcmd__settings__subcmd__help,close)
                cmd="mshellctl__subcmd__settings__subcmd__help__subcmd__close"
                ;;
            mshellctl__subcmd__settings__subcmd__help,help)
                cmd="mshellctl__subcmd__settings__subcmd__help__subcmd__help"
                ;;
            mshellctl__subcmd__settings__subcmd__help,open)
                cmd="mshellctl__subcmd__settings__subcmd__help__subcmd__open"
                ;;
            mshellctl__subcmd__theme,get)
                cmd="mshellctl__subcmd__theme__subcmd__get"
                ;;
            mshellctl__subcmd__theme,help)
                cmd="mshellctl__subcmd__theme__subcmd__help"
                ;;
            mshellctl__subcmd__theme,list)
                cmd="mshellctl__subcmd__theme__subcmd__list"
                ;;
            mshellctl__subcmd__theme,set)
                cmd="mshellctl__subcmd__theme__subcmd__set"
                ;;
            mshellctl__subcmd__theme__subcmd__help,get)
                cmd="mshellctl__subcmd__theme__subcmd__help__subcmd__get"
                ;;
            mshellctl__subcmd__theme__subcmd__help,help)
                cmd="mshellctl__subcmd__theme__subcmd__help__subcmd__help"
                ;;
            mshellctl__subcmd__theme__subcmd__help,list)
                cmd="mshellctl__subcmd__theme__subcmd__help__subcmd__list"
                ;;
            mshellctl__subcmd__theme__subcmd__help,set)
                cmd="mshellctl__subcmd__theme__subcmd__help__subcmd__set"
                ;;
            mshellctl__subcmd__vpn,connect)
                cmd="mshellctl__subcmd__vpn__subcmd__connect"
                ;;
            mshellctl__subcmd__vpn,disconnect)
                cmd="mshellctl__subcmd__vpn__subcmd__disconnect"
                ;;
            mshellctl__subcmd__vpn,fastest)
                cmd="mshellctl__subcmd__vpn__subcmd__fastest"
                ;;
            mshellctl__subcmd__vpn,help)
                cmd="mshellctl__subcmd__vpn__subcmd__help"
                ;;
            mshellctl__subcmd__vpn,menu)
                cmd="mshellctl__subcmd__vpn__subcmd__menu"
                ;;
            mshellctl__subcmd__vpn,random)
                cmd="mshellctl__subcmd__vpn__subcmd__random"
                ;;
            mshellctl__subcmd__vpn,reconnect)
                cmd="mshellctl__subcmd__vpn__subcmd__reconnect"
                ;;
            mshellctl__subcmd__vpn,status)
                cmd="mshellctl__subcmd__vpn__subcmd__status"
                ;;
            mshellctl__subcmd__vpn,toggle)
                cmd="mshellctl__subcmd__vpn__subcmd__toggle"
                ;;
            mshellctl__subcmd__vpn__subcmd__help,connect)
                cmd="mshellctl__subcmd__vpn__subcmd__help__subcmd__connect"
                ;;
            mshellctl__subcmd__vpn__subcmd__help,disconnect)
                cmd="mshellctl__subcmd__vpn__subcmd__help__subcmd__disconnect"
                ;;
            mshellctl__subcmd__vpn__subcmd__help,fastest)
                cmd="mshellctl__subcmd__vpn__subcmd__help__subcmd__fastest"
                ;;
            mshellctl__subcmd__vpn__subcmd__help,help)
                cmd="mshellctl__subcmd__vpn__subcmd__help__subcmd__help"
                ;;
            mshellctl__subcmd__vpn__subcmd__help,menu)
                cmd="mshellctl__subcmd__vpn__subcmd__help__subcmd__menu"
                ;;
            mshellctl__subcmd__vpn__subcmd__help,random)
                cmd="mshellctl__subcmd__vpn__subcmd__help__subcmd__random"
                ;;
            mshellctl__subcmd__vpn__subcmd__help,reconnect)
                cmd="mshellctl__subcmd__vpn__subcmd__help__subcmd__reconnect"
                ;;
            mshellctl__subcmd__vpn__subcmd__help,status)
                cmd="mshellctl__subcmd__vpn__subcmd__help__subcmd__status"
                ;;
            mshellctl__subcmd__vpn__subcmd__help,toggle)
                cmd="mshellctl__subcmd__vpn__subcmd__help__subcmd__toggle"
                ;;
            mshellctl__subcmd__wallpaper,help)
                cmd="mshellctl__subcmd__wallpaper__subcmd__help"
                ;;
            mshellctl__subcmd__wallpaper,next)
                cmd="mshellctl__subcmd__wallpaper__subcmd__next"
                ;;
            mshellctl__subcmd__wallpaper,prev)
                cmd="mshellctl__subcmd__wallpaper__subcmd__prev"
                ;;
            mshellctl__subcmd__wallpaper,random)
                cmd="mshellctl__subcmd__wallpaper__subcmd__random"
                ;;
            mshellctl__subcmd__wallpaper__subcmd__help,help)
                cmd="mshellctl__subcmd__wallpaper__subcmd__help__subcmd__help"
                ;;
            mshellctl__subcmd__wallpaper__subcmd__help,next)
                cmd="mshellctl__subcmd__wallpaper__subcmd__help__subcmd__next"
                ;;
            mshellctl__subcmd__wallpaper__subcmd__help,prev)
                cmd="mshellctl__subcmd__wallpaper__subcmd__help__subcmd__prev"
                ;;
            mshellctl__subcmd__wallpaper__subcmd__help,random)
                cmd="mshellctl__subcmd__wallpaper__subcmd__help__subcmd__random"
                ;;
            *)
                ;;
        esac
    done

    case "${cmd}" in
        mshellctl)
            opts="-h -V --help --version quit inspect set-wallpaper menu bar hidden-bar audio bluetooth media brightness log dock lock session notification settings wizard wallpaper theme plugin screenshot screenrecord clipboard toast gamemode calendar vpn power layout osk color play doctor completions help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 1 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__audio)
            opts="-h --help list status volume-up volume-down volume mute output input switch switch-mic route-next mic-up mic-down mic mic-mute help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__audio__subcmd__help)
            opts="list status volume-up volume-down volume mute output input switch switch-mic route-next mic-up mic-down mic mic-mute help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__audio__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__audio__subcmd__help__subcmd__input)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__audio__subcmd__help__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__audio__subcmd__help__subcmd__mic)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__audio__subcmd__help__subcmd__mic__subcmd__down)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__audio__subcmd__help__subcmd__mic__subcmd__mute)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__audio__subcmd__help__subcmd__mic__subcmd__up)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__audio__subcmd__help__subcmd__mute)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__audio__subcmd__help__subcmd__output)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__audio__subcmd__help__subcmd__route__subcmd__next)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__audio__subcmd__help__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__audio__subcmd__help__subcmd__switch)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__audio__subcmd__help__subcmd__switch__subcmd__mic)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__audio__subcmd__help__subcmd__volume)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__audio__subcmd__help__subcmd__volume__subcmd__down)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__audio__subcmd__help__subcmd__volume__subcmd__up)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__audio__subcmd__input)
            opts="-h --help <TARGET>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__audio__subcmd__list)
            opts="-h --json --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__audio__subcmd__mic)
            opts="-h --help <PERCENT>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__audio__subcmd__mic__subcmd__down)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__audio__subcmd__mic__subcmd__mute)
            opts="-h --help on off"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__audio__subcmd__mic__subcmd__up)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__audio__subcmd__mute)
            opts="-h --help on off"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__audio__subcmd__output)
            opts="-h --help <TARGET>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__audio__subcmd__route__subcmd__next)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__audio__subcmd__status)
            opts="-h --json --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__audio__subcmd__switch)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__audio__subcmd__switch__subcmd__mic)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__audio__subcmd__volume)
            opts="-h --help <PERCENT>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__audio__subcmd__volume__subcmd__down)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__audio__subcmd__volume__subcmd__up)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__bar)
            opts="-h --help top bottom toggle-all toggle reveal-all show show-all reveal hide-all hide help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__bar__subcmd__bottom)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__bar__subcmd__help)
            opts="top bottom toggle-all reveal-all hide-all help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__bar__subcmd__help__subcmd__bottom)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__bar__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__bar__subcmd__help__subcmd__hide__subcmd__all)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__bar__subcmd__help__subcmd__reveal__subcmd__all)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__bar__subcmd__help__subcmd__toggle__subcmd__all)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__bar__subcmd__help__subcmd__top)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__bar__subcmd__hide__subcmd__all)
            opts="-x -h --exclude --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__bar__subcmd__reveal__subcmd__all)
            opts="-x -h --exclude --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__bar__subcmd__toggle__subcmd__all)
            opts="-x -h --exclude --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__bar__subcmd__top)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__bluetooth)
            opts="-h --help toggle connect disconnect help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__bluetooth__subcmd__connect)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__bluetooth__subcmd__disconnect)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__bluetooth__subcmd__help)
            opts="toggle connect disconnect help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__bluetooth__subcmd__help__subcmd__connect)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__bluetooth__subcmd__help__subcmd__disconnect)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__bluetooth__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__bluetooth__subcmd__help__subcmd__toggle)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__bluetooth__subcmd__toggle)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__brightness)
            opts="-h --help up down help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__brightness__subcmd__down)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__brightness__subcmd__help)
            opts="up down help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__brightness__subcmd__help__subcmd__down)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__brightness__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__brightness__subcmd__help__subcmd__up)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__brightness__subcmd__up)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__calendar)
            opts="-h --help today agenda on account help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__calendar__subcmd__account)
            opts="-h --help [ARGS]..."
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__calendar__subcmd__agenda)
            opts="-h --help [DAYS]"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__calendar__subcmd__help)
            opts="today agenda on account help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__calendar__subcmd__help__subcmd__account)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__calendar__subcmd__help__subcmd__agenda)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__calendar__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__calendar__subcmd__help__subcmd__on)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__calendar__subcmd__help__subcmd__today)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__calendar__subcmd__on)
            opts="-h --help <DATE>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__calendar__subcmd__today)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__clipboard)
            opts="-h --help list copy pin unpin delete clear wipe help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__clipboard__subcmd__clear)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__clipboard__subcmd__copy)
            opts="-h --help <ID>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__clipboard__subcmd__delete)
            opts="-h --help <ID>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__clipboard__subcmd__help)
            opts="list copy pin unpin delete clear wipe help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__clipboard__subcmd__help__subcmd__clear)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__clipboard__subcmd__help__subcmd__copy)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__clipboard__subcmd__help__subcmd__delete)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__clipboard__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__clipboard__subcmd__help__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__clipboard__subcmd__help__subcmd__pin)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__clipboard__subcmd__help__subcmd__unpin)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__clipboard__subcmd__help__subcmd__wipe)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__clipboard__subcmd__list)
            opts="-h --json --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__clipboard__subcmd__pin)
            opts="-h --help <ID>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__clipboard__subcmd__unpin)
            opts="-h --help <ID>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__clipboard__subcmd__wipe)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__color)
            opts="-h --copy --notify --format --lowercase --no-zoom --quiet --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "hex rgb hsl cmyk" -- "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__completions)
            opts="-h --help bash elvish fish powershell zsh"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__dock)
            opts="-h --help toggle show hide activate help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__dock__subcmd__activate)
            opts="-h --help <INDEX>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__dock__subcmd__help)
            opts="toggle show hide activate help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__dock__subcmd__help__subcmd__activate)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__dock__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__dock__subcmd__help__subcmd__hide)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__dock__subcmd__help__subcmd__show)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__dock__subcmd__help__subcmd__toggle)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__dock__subcmd__hide)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__dock__subcmd__show)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__dock__subcmd__toggle)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__doctor)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__gamemode)
            opts="-h --help on off toggle status"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help)
            opts="quit inspect set-wallpaper menu bar hidden-bar audio bluetooth media brightness log dock lock session notification settings wizard wallpaper theme plugin screenshot screenrecord clipboard toast gamemode calendar vpn power layout osk color play doctor completions help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__audio)
            opts="list status volume-up volume-down volume mute output input switch switch-mic route-next mic-up mic-down mic mic-mute"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__audio__subcmd__input)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__audio__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__audio__subcmd__mic)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__audio__subcmd__mic__subcmd__down)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__audio__subcmd__mic__subcmd__mute)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__audio__subcmd__mic__subcmd__up)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__audio__subcmd__mute)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__audio__subcmd__output)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__audio__subcmd__route__subcmd__next)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__audio__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__audio__subcmd__switch)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__audio__subcmd__switch__subcmd__mic)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__audio__subcmd__volume)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__audio__subcmd__volume__subcmd__down)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__audio__subcmd__volume__subcmd__up)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__bar)
            opts="top bottom toggle-all reveal-all hide-all"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__bar__subcmd__bottom)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__bar__subcmd__hide__subcmd__all)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__bar__subcmd__reveal__subcmd__all)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__bar__subcmd__toggle__subcmd__all)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__bar__subcmd__top)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__bluetooth)
            opts="toggle connect disconnect"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__bluetooth__subcmd__connect)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__bluetooth__subcmd__disconnect)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__bluetooth__subcmd__toggle)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__brightness)
            opts="up down"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__brightness__subcmd__down)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__brightness__subcmd__up)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__calendar)
            opts="today agenda on account"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__calendar__subcmd__account)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__calendar__subcmd__agenda)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__calendar__subcmd__on)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__calendar__subcmd__today)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__clipboard)
            opts="list copy pin unpin delete clear wipe"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__clipboard__subcmd__clear)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__clipboard__subcmd__copy)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__clipboard__subcmd__delete)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__clipboard__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__clipboard__subcmd__pin)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__clipboard__subcmd__unpin)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__clipboard__subcmd__wipe)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__color)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__completions)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__dock)
            opts="toggle show hide activate"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__dock__subcmd__activate)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__dock__subcmd__hide)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__dock__subcmd__show)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__dock__subcmd__toggle)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__doctor)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__gamemode)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__hidden__subcmd__bar)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__inspect)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__layout)
            opts="list current set next prev preview pick"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__layout__subcmd__current)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__layout__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__layout__subcmd__next)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__layout__subcmd__pick)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__layout__subcmd__prev)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__layout__subcmd__preview)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__layout__subcmd__set)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__lock)
            opts="activate check"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__lock__subcmd__activate)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__lock__subcmd__check)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__log)
            opts="level enable disable path open"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__log__subcmd__disable)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__log__subcmd__enable)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__log__subcmd__level)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__log__subcmd__open)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__log__subcmd__path)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__media)
            opts="toggle next prev status list"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__media__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__media__subcmd__next)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__media__subcmd__prev)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__media__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__media__subcmd__toggle)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu)
            opts="app-launcher clipboard clock notifications screenshot wallpaper ufw privacy bluetooth cpu-dashboard audio-dashboard audio-route system-update valent keep-awake twilight margo-layout weather keybinds alarm-clock control-center ssh-sessions vpn dns ai podman notes ip network power media-player lyrics session mdash plugin close-all"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__ai)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__alarm__subcmd__clock)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__app__subcmd__launcher)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__audio__subcmd__dashboard)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__audio__subcmd__route)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__bluetooth)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__clipboard)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__clock)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__close__subcmd__all)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__control__subcmd__center)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__cpu__subcmd__dashboard)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__dns)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__ip)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__keep__subcmd__awake)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__keybinds)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__lyrics)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__margo__subcmd__layout)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__mdash)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__media__subcmd__player)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__network)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__notes)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__notifications)
            opts="clears read dnd count"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__notifications__subcmd__clears)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__notifications__subcmd__count)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__notifications__subcmd__dnd)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__notifications__subcmd__read)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__plugin)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__podman)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__power)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__privacy)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__screenshot)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__session)
            opts="lock logout suspend reboot shutdown"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__session__subcmd__lock)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__session__subcmd__logout)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__session__subcmd__reboot)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__session__subcmd__shutdown)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__session__subcmd__suspend)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__ssh__subcmd__sessions)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__system__subcmd__update)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__twilight)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__ufw)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__valent)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__vpn)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__wallpaper)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__menu__subcmd__weather)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__notification)
            opts="open clear read dnd count"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__notification__subcmd__clear)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__notification__subcmd__count)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__notification__subcmd__dnd)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__notification__subcmd__open)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__notification__subcmd__read)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__osk)
            opts="show hide toggle"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__osk__subcmd__hide)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__osk__subcmd__show)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__osk__subcmd__toggle)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__play)
            opts="start toggle play stop snap pin focus wallpaper"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__play__subcmd__focus)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__play__subcmd__pin)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__play__subcmd__play)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__play__subcmd__snap)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__play__subcmd__start)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__play__subcmd__stop)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__play__subcmd__toggle)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__play__subcmd__wallpaper)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__plugin)
            opts="list reload keybind"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__plugin__subcmd__keybind)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__plugin__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__plugin__subcmd__reload)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__power)
            opts="status cycle set pause resume auto"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__power__subcmd__auto)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__power__subcmd__cycle)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__power__subcmd__pause)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__power__subcmd__resume)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__power__subcmd__set)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__power__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__quit)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__screenrecord)
            opts="start stop toggle"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__screenrecord__subcmd__start)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__screenrecord__subcmd__stop)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__screenrecord__subcmd__toggle)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__screenshot)
            opts="region window output full select-region"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__screenshot__subcmd__full)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__screenshot__subcmd__output)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__screenshot__subcmd__region)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__screenshot__subcmd__select__subcmd__region)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__screenshot__subcmd__window)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__session)
            opts="menu lock logout suspend reboot shutdown"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__session__subcmd__lock)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__session__subcmd__logout)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__session__subcmd__menu)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__session__subcmd__reboot)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__session__subcmd__shutdown)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__session__subcmd__suspend)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__set__subcmd__wallpaper)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__settings)
            opts="open close"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__settings__subcmd__close)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__settings__subcmd__open)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__theme)
            opts="list get set"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__theme__subcmd__get)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__theme__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__theme__subcmd__set)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__toast)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__vpn)
            opts="status connect disconnect toggle reconnect random fastest menu"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__vpn__subcmd__connect)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__vpn__subcmd__disconnect)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__vpn__subcmd__fastest)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__vpn__subcmd__menu)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__vpn__subcmd__random)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__vpn__subcmd__reconnect)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__vpn__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__vpn__subcmd__toggle)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__wallpaper)
            opts="next prev random"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__wallpaper__subcmd__next)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__wallpaper__subcmd__prev)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__wallpaper__subcmd__random)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__help__subcmd__wizard)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__hidden__subcmd__bar)
            opts="-h --help <ACTION> [NAME]"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__inspect)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__layout)
            opts="-h --help list current set next prev preview pick help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__layout__subcmd__current)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__layout__subcmd__help)
            opts="list current set next prev preview pick help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__layout__subcmd__help__subcmd__current)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__layout__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__layout__subcmd__help__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__layout__subcmd__help__subcmd__next)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__layout__subcmd__help__subcmd__pick)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__layout__subcmd__help__subcmd__prev)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__layout__subcmd__help__subcmd__preview)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__layout__subcmd__help__subcmd__set)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__layout__subcmd__list)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__layout__subcmd__next)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__layout__subcmd__pick)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__layout__subcmd__prev)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__layout__subcmd__preview)
            opts="-h --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__layout__subcmd__set)
            opts="-h --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__lock)
            opts="-h --help activate check help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__lock__subcmd__activate)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__lock__subcmd__check)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__lock__subcmd__help)
            opts="activate check help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__lock__subcmd__help__subcmd__activate)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__lock__subcmd__help__subcmd__check)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__lock__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__log)
            opts="-h --help level enable disable path open help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__log__subcmd__disable)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__log__subcmd__enable)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__log__subcmd__help)
            opts="level enable disable path open help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__log__subcmd__help__subcmd__disable)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__log__subcmd__help__subcmd__enable)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__log__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__log__subcmd__help__subcmd__level)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__log__subcmd__help__subcmd__open)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__log__subcmd__help__subcmd__path)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__log__subcmd__level)
            opts="-h --help <LEVEL>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__log__subcmd__open)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__log__subcmd__path)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__media)
            opts="-h --help toggle next prev status list help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__media__subcmd__help)
            opts="toggle next prev status list help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__media__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__media__subcmd__help__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__media__subcmd__help__subcmd__next)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__media__subcmd__help__subcmd__prev)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__media__subcmd__help__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__media__subcmd__help__subcmd__toggle)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__media__subcmd__list)
            opts="-h --json --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__media__subcmd__next)
            opts="-h --help [PLAYER]"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__media__subcmd__prev)
            opts="-h --help [PLAYER]"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__media__subcmd__status)
            opts="-h --json --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__media__subcmd__toggle)
            opts="-h --help [PLAYER]"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu)
            opts="-h --help app-launcher clipboard clock notifications screenshot wallpaper ufw privacy bluetooth cpu-dashboard audio-dashboard audio-route system-update valent keep-awake twilight margo-layout weather keybinds alarm-clock control-center ssh-sessions vpn dns ai podman notes ip network power media-player lyrics session mdash plugin close-all help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__ai)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__alarm__subcmd__clock)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__app__subcmd__launcher)
            opts="-h --tab --list-tabs --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --tab)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__audio__subcmd__dashboard)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__audio__subcmd__route)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__bluetooth)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__clipboard)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__clock)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__close__subcmd__all)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__control__subcmd__center)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__cpu__subcmd__dashboard)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__dns)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help)
            opts="app-launcher clipboard clock notifications screenshot wallpaper ufw privacy bluetooth cpu-dashboard audio-dashboard audio-route system-update valent keep-awake twilight margo-layout weather keybinds alarm-clock control-center ssh-sessions vpn dns ai podman notes ip network power media-player lyrics session mdash plugin close-all help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__ai)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__alarm__subcmd__clock)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__app__subcmd__launcher)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__audio__subcmd__dashboard)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__audio__subcmd__route)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__bluetooth)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__clipboard)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__clock)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__close__subcmd__all)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__control__subcmd__center)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__cpu__subcmd__dashboard)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__dns)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__ip)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__keep__subcmd__awake)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__keybinds)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__lyrics)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__margo__subcmd__layout)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__mdash)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__media__subcmd__player)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__network)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__notes)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__notifications)
            opts="clears read dnd count"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__notifications__subcmd__clears)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__notifications__subcmd__count)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__notifications__subcmd__dnd)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__notifications__subcmd__read)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__plugin)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__podman)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__power)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__privacy)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__screenshot)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__session)
            opts="lock logout suspend reboot shutdown"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__session__subcmd__lock)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__session__subcmd__logout)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__session__subcmd__reboot)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__session__subcmd__shutdown)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__session__subcmd__suspend)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__ssh__subcmd__sessions)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__system__subcmd__update)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__twilight)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__ufw)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__valent)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__vpn)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__wallpaper)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__help__subcmd__weather)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__ip)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__keep__subcmd__awake)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__keybinds)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__lyrics)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__margo__subcmd__layout)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__mdash)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__media__subcmd__player)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__network)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__notes)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__notifications)
            opts="-h --help clears read dnd count help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__notifications__subcmd__clears)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__notifications__subcmd__count)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__notifications__subcmd__dnd)
            opts="-h --help on off"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__notifications__subcmd__help)
            opts="clears read dnd count help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__notifications__subcmd__help__subcmd__clears)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__notifications__subcmd__help__subcmd__count)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__notifications__subcmd__help__subcmd__dnd)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__notifications__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__notifications__subcmd__help__subcmd__read)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__notifications__subcmd__read)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__plugin)
            opts="-h --help <KEY>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__podman)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__power)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__privacy)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__screenshot)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__session)
            opts="-h --help lock logout suspend reboot shutdown help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__session__subcmd__help)
            opts="lock logout suspend reboot shutdown help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__session__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__session__subcmd__help__subcmd__lock)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__session__subcmd__help__subcmd__logout)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__session__subcmd__help__subcmd__reboot)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__session__subcmd__help__subcmd__shutdown)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__session__subcmd__help__subcmd__suspend)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__session__subcmd__lock)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__session__subcmd__logout)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__session__subcmd__reboot)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__session__subcmd__shutdown)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__session__subcmd__suspend)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__ssh__subcmd__sessions)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__system__subcmd__update)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__twilight)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__ufw)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__valent)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__vpn)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__wallpaper)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__menu__subcmd__weather)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__notification)
            opts="-h --help open clear read dnd count help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__notification__subcmd__clear)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__notification__subcmd__count)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__notification__subcmd__dnd)
            opts="-h --help on off"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__notification__subcmd__help)
            opts="open clear read dnd count help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__notification__subcmd__help__subcmd__clear)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__notification__subcmd__help__subcmd__count)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__notification__subcmd__help__subcmd__dnd)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__notification__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__notification__subcmd__help__subcmd__open)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__notification__subcmd__help__subcmd__read)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__notification__subcmd__open)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__notification__subcmd__read)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__osk)
            opts="-h --help show hide toggle help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__osk__subcmd__help)
            opts="show hide toggle help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__osk__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__osk__subcmd__help__subcmd__hide)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__osk__subcmd__help__subcmd__show)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__osk__subcmd__help__subcmd__toggle)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__osk__subcmd__hide)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__osk__subcmd__show)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__osk__subcmd__toggle)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__play)
            opts="-h --help start toggle play stop snap pin focus wallpaper help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__play__subcmd__focus)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__play__subcmd__help)
            opts="start toggle play stop snap pin focus wallpaper help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__play__subcmd__help__subcmd__focus)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__play__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__play__subcmd__help__subcmd__pin)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__play__subcmd__help__subcmd__play)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__play__subcmd__help__subcmd__snap)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__play__subcmd__help__subcmd__start)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__play__subcmd__help__subcmd__stop)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__play__subcmd__help__subcmd__toggle)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__play__subcmd__help__subcmd__wallpaper)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__play__subcmd__pin)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__play__subcmd__play)
            opts="-h --help [TARGET]"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__play__subcmd__snap)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__play__subcmd__start)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__play__subcmd__stop)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__play__subcmd__toggle)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__play__subcmd__wallpaper)
            opts="-h --help [ARGS]..."
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__plugin)
            opts="-h --help list reload keybind help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__plugin__subcmd__help)
            opts="list reload keybind help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__plugin__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__plugin__subcmd__help__subcmd__keybind)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__plugin__subcmd__help__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__plugin__subcmd__help__subcmd__reload)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__plugin__subcmd__keybind)
            opts="-h --help <KEY> <ID>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__plugin__subcmd__list)
            opts="-h --names --enabled --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__plugin__subcmd__reload)
            opts="-h --help <KEY>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__power)
            opts="-h --help status cycle set pause resume auto help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__power__subcmd__auto)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__power__subcmd__cycle)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__power__subcmd__help)
            opts="status cycle set pause resume auto help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__power__subcmd__help__subcmd__auto)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__power__subcmd__help__subcmd__cycle)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__power__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__power__subcmd__help__subcmd__pause)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__power__subcmd__help__subcmd__resume)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__power__subcmd__help__subcmd__set)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__power__subcmd__help__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__power__subcmd__pause)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__power__subcmd__resume)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__power__subcmd__set)
            opts="-h --help performance balanced power-saver"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__power__subcmd__status)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__quit)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__screenrecord)
            opts="-h --help start stop toggle help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__screenrecord__subcmd__help)
            opts="start stop toggle help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__screenrecord__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__screenrecord__subcmd__help__subcmd__start)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__screenrecord__subcmd__help__subcmd__stop)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__screenrecord__subcmd__help__subcmd__toggle)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__screenrecord__subcmd__start)
            opts="-h --audio --help region window output full"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --audio)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__screenrecord__subcmd__stop)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__screenrecord__subcmd__toggle)
            opts="-h --audio --help region window output full"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --audio)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__screenshot)
            opts="-h --help region window output full select-region help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__screenshot__subcmd__full)
            opts="-d -h --copy --save --edit --delay --help [EDITOR]"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --delay)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -d)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__screenshot__subcmd__help)
            opts="region window output full select-region help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__screenshot__subcmd__help__subcmd__full)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__screenshot__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__screenshot__subcmd__help__subcmd__output)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__screenshot__subcmd__help__subcmd__region)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__screenshot__subcmd__help__subcmd__select__subcmd__region)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__screenshot__subcmd__help__subcmd__window)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__screenshot__subcmd__output)
            opts="-d -h --copy --save --edit --delay --help [EDITOR]"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --delay)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -d)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__screenshot__subcmd__region)
            opts="-d -h --copy --save --edit --delay --help [EDITOR]"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --delay)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -d)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__screenshot__subcmd__select__subcmd__region)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__screenshot__subcmd__window)
            opts="-d -h --copy --save --edit --delay --help [EDITOR]"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --delay)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -d)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__session)
            opts="-h --help menu lock logout suspend reboot shutdown help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__session__subcmd__help)
            opts="menu lock logout suspend reboot shutdown help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__session__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__session__subcmd__help__subcmd__lock)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__session__subcmd__help__subcmd__logout)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__session__subcmd__help__subcmd__menu)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__session__subcmd__help__subcmd__reboot)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__session__subcmd__help__subcmd__shutdown)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__session__subcmd__help__subcmd__suspend)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__session__subcmd__lock)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__session__subcmd__logout)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__session__subcmd__menu)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__session__subcmd__reboot)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__session__subcmd__shutdown)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__session__subcmd__suspend)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__set__subcmd__wallpaper)
            opts="-h --help <PATH>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__settings)
            opts="-h --help open close help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__settings__subcmd__close)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__settings__subcmd__help)
            opts="open close help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__settings__subcmd__help__subcmd__close)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__settings__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__settings__subcmd__help__subcmd__open)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__settings__subcmd__open)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__theme)
            opts="-h --help list get set help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__theme__subcmd__get)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__theme__subcmd__help)
            opts="list get set help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__theme__subcmd__help__subcmd__get)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__theme__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__theme__subcmd__help__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__theme__subcmd__help__subcmd__set)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__theme__subcmd__list)
            opts="-h --names-only --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__theme__subcmd__set)
            opts="-h --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__toast)
            opts="-h --icon --severity --help <TITLE> [BODY]"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --icon)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --severity)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__vpn)
            opts="-h --help status connect disconnect toggle reconnect random fastest menu help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__vpn__subcmd__connect)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__vpn__subcmd__disconnect)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__vpn__subcmd__fastest)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__vpn__subcmd__help)
            opts="status connect disconnect toggle reconnect random fastest menu help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__vpn__subcmd__help__subcmd__connect)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__vpn__subcmd__help__subcmd__disconnect)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__vpn__subcmd__help__subcmd__fastest)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__vpn__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__vpn__subcmd__help__subcmd__menu)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__vpn__subcmd__help__subcmd__random)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__vpn__subcmd__help__subcmd__reconnect)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__vpn__subcmd__help__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__vpn__subcmd__help__subcmd__toggle)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__vpn__subcmd__menu)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__vpn__subcmd__random)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__vpn__subcmd__reconnect)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__vpn__subcmd__status)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__vpn__subcmd__toggle)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__wallpaper)
            opts="-h --help next prev random help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__wallpaper__subcmd__help)
            opts="next prev random help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__wallpaper__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__wallpaper__subcmd__help__subcmd__next)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__wallpaper__subcmd__help__subcmd__prev)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__wallpaper__subcmd__help__subcmd__random)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__wallpaper__subcmd__next)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__wallpaper__subcmd__prev)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__wallpaper__subcmd__random)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        mshellctl__subcmd__wizard)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
    esac
}

if [[ "${BASH_VERSINFO[0]}" -eq 4 && "${BASH_VERSINFO[1]}" -ge 4 || "${BASH_VERSINFO[0]}" -gt 4 ]]; then
    complete -F _mshellctl -o nosort -o bashdefault -o default mshellctl
else
    complete -F _mshellctl -o bashdefault -o default mshellctl
fi
