import gettext
from gi.repository import Gtk, Adw, GLib, Gio

_ = gettext.gettext

class WhatsNewWindow(Adw.Window):
    def __init__(self, parent_win, version, date_str, releases_list):
        super().__init__(
            transient_for=parent_win,
            modal=True,
            default_width=440,
            default_height=580,
            resizable=False,
            title=""
        )
        
        main_box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        
        # HeaderBar
        header_bar = Adw.HeaderBar(show_end_title_buttons=True)
        header_bar.add_css_class("flat")
        main_box.append(header_bar)
        
        # Header Title & Date (H1)
        title_box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4, margin_top=0, margin_bottom=12)
        
        rels_to_show = releases_list if releases_list else []
        is_multi_release = len(rels_to_show) > 1

        if not is_multi_release:
            title_text = f"<span size='18000' weight='ultrabold'>What's New in {version}?</span>"
        else:
            title_text = "<span size='18000' weight='ultrabold'>What's New in Whisp?</span>"
            
        title_lbl = Gtk.Label(
            label=title_text,
            use_markup=True,
            justify=Gtk.Justification.CENTER
        )
        
        formatted_date = ""
        if date_str:
            try:
                from datetime import datetime
                d_obj = datetime.strptime(date_str, "%Y-%m-%d")
                formatted_date = d_obj.strftime("%B %d, %Y").replace(" 0", " ")
            except Exception:
                formatted_date = date_str
                
        if not is_multi_release:
            subtitle_text = _("Released {date}").format(date=formatted_date) if formatted_date else ""
        else:
            subtitle_text = _("Updates since your last visit")
            
        subtitle_lbl = Gtk.Label(
            label=f"<span size='small' foreground='gray'>{subtitle_text}</span>" if subtitle_text else "",
            use_markup=True,
            justify=Gtk.Justification.CENTER
        )
        
        title_box.append(title_lbl)
        if subtitle_text:
            title_box.append(subtitle_lbl)
        main_box.append(title_box)
        
        # Scrollable Content
        scroll = Gtk.ScrolledWindow()
        scroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        scroll.set_vexpand(True)
        
        content_box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=0, margin_start=24, margin_end=24, margin_bottom=20)
        
        for r_idx, rel in enumerate(rels_to_show):
            rel_ver = rel.get("version", "")
            rel_date = rel.get("date", "")
            
            if is_multi_release:
                rel_formatted_date = ""
                if rel_date:
                    try:
                        from datetime import datetime
                        d_obj = datetime.strptime(rel_date, "%Y-%m-%d")
                        rel_formatted_date = d_obj.strftime("%B %d, %Y").replace(" 0", " ")
                    except Exception:
                        rel_formatted_date = rel_date
                        
                ver_header_lbl = Gtk.Label(
                    label=f"<span size='13500' weight='bold'>Version {rel_ver}</span> <span size='small' foreground='gray'>({rel_formatted_date})</span>",
                    use_markup=True,
                    halign=Gtk.Align.START,
                    margin_top=14 if r_idx > 0 else 4,
                    margin_bottom=6
                )
                content_box.append(ver_header_lbl)

            features = rel.get("features", [])
            bug_fixes = rel.get("bug_fixes", [])

            # Features Section
            if features:
                feat_title = Gtk.Label(
                    label=f"<b>{_('New Features')}</b>",
                    use_markup=True,
                    halign=Gtk.Align.START,
                    margin_top=8,
                    margin_bottom=6
                )
                content_box.append(feat_title)
                
                feat_box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6, margin_bottom=12)
                for item in features:
                    if ":" in item:
                        t, _sep, r = item.partition(":")
                        b_t = GLib.markup_escape_text(t.strip())
                        b_r = GLib.markup_escape_text(r)
                        item_markup = f"• <b>{b_t}:</b>{b_r}"
                    else:
                        escaped = GLib.markup_escape_text(item)
                        item_markup = f"• {escaped}"
                    
                    item_lbl = Gtk.Label(
                        label=item_markup,
                        use_markup=True,
                        wrap=True,
                        justify=Gtk.Justification.LEFT,
                        halign=Gtk.Align.START
                    )
                    feat_box.append(item_lbl)
                content_box.append(feat_box)

            # Bug Fixes Section
            if bug_fixes:
                fix_title = Gtk.Label(
                    label=f"<b>{_('Bug Fixes')}</b>",
                    use_markup=True,
                    halign=Gtk.Align.START,
                    margin_top=10 if features else 8,
                    margin_bottom=6
                )
                content_box.append(fix_title)
                
                fix_box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6, margin_bottom=12)
                for item in bug_fixes:
                    if ":" in item:
                        t, _sep, r = item.partition(":")
                        b_t = GLib.markup_escape_text(t.strip())
                        b_r = GLib.markup_escape_text(r)
                        item_markup = f"• <b>{b_t}:</b>{b_r}"
                    else:
                        escaped = GLib.markup_escape_text(item)
                        item_markup = f"• {escaped}"
                    
                    item_lbl = Gtk.Label(
                        label=item_markup,
                        use_markup=True,
                        wrap=True,
                        justify=Gtk.Justification.LEFT,
                        halign=Gtk.Align.START
                    )
                    fix_box.append(item_lbl)
                content_box.append(fix_box)

            if is_multi_release and r_idx < len(rels_to_show) - 1:
                sep = Gtk.Separator(orientation=Gtk.Orientation.HORIZONTAL)
                sep.set_margin_top(12)
                sep.set_margin_bottom(12)
                content_box.append(sep)

        thanks_lbl = Gtk.Label(
            label=_("<span size='small' foreground='gray'>Thanks for reading, and have a great day!</span>"),
            use_markup=True,
            justify=Gtk.Justification.CENTER,
            margin_top=14,
            margin_bottom=10
        )
        content_box.append(thanks_lbl)
        
        full_changelog_btn = Gtk.Button(label=_("Full Release Notes ↗"))
        full_changelog_btn.add_css_class("pill")
        full_changelog_btn.set_halign(Gtk.Align.CENTER)
        full_changelog_btn.set_margin_top(2)
        full_changelog_btn.set_margin_bottom(16)
        
        def on_changelog_click(btn):
            if hasattr(Gtk, "UriLauncher"):
                launcher = Gtk.UriLauncher.new("https://github.com/tanaybhomia/Whisp/releases")
                launcher.launch(self, None, None)
            else:
                Gio.AppInfo.launch_default_for_uri("https://github.com/tanaybhomia/Whisp/releases", None)
                
        full_changelog_btn.connect("clicked", on_changelog_click)
        content_box.append(full_changelog_btn)
        
        scroll.set_child(content_box)
        main_box.append(scroll)
        
        # Bottom Support Banner Card
        support_card = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
        support_card.add_css_class("whisp-support-card")
        
        support_title = Gtk.Label(
            label=_("This release was made possible by users like you!"),
            use_markup=True,
            wrap=True,
            justify=Gtk.Justification.CENTER,
            halign=Gtk.Align.CENTER
        )
        support_title.add_css_class("support-title")
        
        support_msg = Gtk.Label(
            label=_("I love making Whisp, but I cannot do it alone. Help support further development by donating."),
            use_markup=True,
            wrap=True,
            justify=Gtk.Justification.CENTER,
            halign=Gtk.Align.CENTER
        )
        support_msg.add_css_class("support-text")
        
        donate_btn = Gtk.Button(label=_("❤ Support Whisp"))
        donate_btn.add_css_class("whisp-support-btn")
        donate_btn.set_halign(Gtk.Align.CENTER)
        
        from whisp.dialogs.about import open_donate_url
        donate_btn.connect("clicked", lambda btn: open_donate_url(self))
        
        support_card.append(support_title)
        support_card.append(support_msg)
        support_card.append(donate_btn)
        
        main_box.append(support_card)
        self.set_content(main_box)
