import os
import re
import sys
import platform
import gettext
import xml.etree.ElementTree as ET
from pathlib import Path
from typing import Any, Tuple, List, Dict, cast

from gi.repository import Gtk, Adw, Gio, GLib

_ = gettext.gettext


def get_latest_release_info(
    last_seen: str = "0.0.0",
    only_latest: bool = False,
    as_list: bool = False
) -> Tuple[str, Any]:
    """Parse application releases from the metainfo.xml file."""
    try:
        meta_path = Path("/app/share/metainfo/io.github.tanaybhomia.Whisp.metainfo.xml")
        if not meta_path.exists():
            meta_path = Path(__file__).parent.parent.parent.parent / "data" / "io.github.tanaybhomia.Whisp.metainfo.xml"

        tree = ET.parse(meta_path)
        root = tree.getroot()
        releases = root.find("releases")

        def parse_ver(v_str: str) -> Tuple[int, ...]:
            try:
                return tuple(map(int, v_str.split(".")))
            except Exception:
                return (0, 0, 0)

        last_seen_tuple = parse_ver(last_seen)

        if releases is not None:
            description_text = ""
            latest_version = "Unknown"
            releases_list: List[Dict[str, Any]] = []

            for i, release in enumerate(releases.findall("release")):
                version = release.attrib.get("version", "Unknown")
                date = release.attrib.get("date", "")
                if i == 0:
                    latest_version = version

                ver_tuple = parse_ver(version)
                if ver_tuple <= last_seen_tuple or (only_latest and i > 0):
                    break

                release_desc = ""
                if not as_list:
                    description_text += f"<span size='large' weight='bold'>v{version}</span>\n"

                features: List[str] = []
                bug_fixes: List[str] = []
                current_cat = None

                desc_node = release.find("description")
                raw_xml = ""
                if desc_node is not None:
                    raw_xml = "".join(ET.tostring(child, encoding="unicode").strip() for child in desc_node)
                    for child in desc_node:
                        if child.tag == "p":
                            text = "".join(child.itertext()).strip()
                            lower_text = text.lower()
                            if "feature" in lower_text:
                                current_cat = "features"
                            elif any(k in lower_text for k in ["fix", "hotfix", "bug", "maintenance"]):
                                current_cat = "bug_fixes"

                            if text:
                                escaped = GLib.markup_escape_text(text)
                                if child.find("em") is not None:
                                    release_desc += f"<b>{escaped}</b>\n\n"
                                else:
                                    release_desc += f"{escaped}\n\n"
                        elif child.tag == "ul":
                            for li in child.findall("li"):
                                li_text = "".join(li.itertext()).strip()
                                if not li_text:
                                    continue
                                lower_li = li_text.lower()
                                if current_cat == "bug_fixes" or any(k in lower_li for k in ["fix", "crash", "resolved", "bug", "stability"]):
                                    bug_fixes.append(li_text)
                                else:
                                    features.append(li_text)

                                if ":" in li_text:
                                    title, _, rest = li_text.partition(":")
                                    b_title = GLib.markup_escape_text(title.strip())
                                    b_rest = GLib.markup_escape_text(rest)
                                    release_desc += f"• <b>{b_title}:</b>{b_rest}\n"
                                else:
                                    escaped = GLib.markup_escape_text(li_text)
                                    release_desc += f"• {escaped}\n"
                            release_desc += "\n"

                releases_list.append({
                    "version": version,
                    "date": date,
                    "raw_xml": raw_xml,
                    "description": release_desc.strip(),
                    "features": features,
                    "bug_fixes": bug_fixes
                })
                description_text += release_desc

            if as_list:
                return latest_version, releases_list
            return latest_version, description_text.strip()
    except Exception:
        pass
    if as_list:
        return "Unknown", []
    return "Unknown", ""


def get_dynamic_version() -> str:
    """Get the current application version dynamically from metadata."""
    version, _desc = get_latest_release_info(only_latest=True)
    return str(version) if version != "Unknown" else "1.0.0"


def open_donate_url(parent_window: Gtk.Window | None = None) -> None:
    """Open the donation web page."""
    donate_url = "https://tanaybhomia.github.io/Whisp/donate.html"
    if hasattr(Gtk, "UriLauncher"):
        launcher = Gtk.UriLauncher.new(donate_url)
        target_parent = parent_window if isinstance(parent_window, Gtk.Window) else None
        launcher.launch(target_parent, None, None)
    else:
        Gio.AppInfo.launch_default_for_uri(donate_url, None)


def show_about_dialog(parent_window: Gtk.Window | None, open_page: str | None = None) -> Adw.AboutDialog:
    """Build and present the Adw.AboutDialog with system debug information and release notes."""
    version = get_dynamic_version()
    app = parent_window.get_application() if parent_window else Gio.Application.get_default()
    app_id = app.get_application_id() if app else ""
    is_dev = (bool(app_id) and app_id.endswith(".Devel")) or "--dev" in sys.argv

    icon_name = "io.github.tanaybhomia.Whisp.Devel" if is_dev else "io.github.tanaybhomia.Whisp"
    app_name = "Whisp (Development)" if is_dev else "Whisp"

    about = Adw.AboutDialog(
        application_name=app_name,
        application_icon=icon_name,
        developer_name="Tanay Bhomia",
        developers=["Tanay Bhomia"],
        version=version,
        website="https://tanaybhomia.github.io/Whisp/",
        issue_url="https://github.com/tanaybhomia/Whisp/issues",
        support_url="https://github.com/tanaybhomia/Whisp/discussions",
        license_type=Gtk.License.GPL_3_0
    )
    translator_credits = _("translator-credits")
    if translator_credits == "translator-credits":
        translator_credits = (
            "Asier Saratsua Garmendia <asiersarasua@ni.eus>\n"
            "Florian Obernberger <florian.obernberger@gmail.com>\n"
            "Guillaume Bernard <contact@guillaume-bernard.fr>\n"
            "Ouiplala <ouiplala@laposte.net>\n"
            "Baurzhan Muftakhidinov <baurthefirst@gmail.com>\n"
            "Martin Srebotnjak <miles@filmsi.net>\n"
            "Anders Jonsson <anders.jonsson@norsjovallen.se>\n"
            "Yuri Chornoivan <yurchor@ukr.net>"
        )
    about.set_translator_credits(translator_credits)
    about.add_credit_section(_("Icon by"), ["gnoman"])
    about.add_acknowledgement_section(None, ["arewelibadwaitayet https://arewelibadwaitayet.com/"])

    if hasattr(about, "add_other_app"):
        about.add_other_app(
            "io.github.tanaybhomia.DeepDive",
            _("Deep Dive"),
            _("Stay focused with strict limits")
        )

    about.add_link(_("Manual"), "https://tanaybhomia.github.io/Whisp/manual.html")
    about.add_link(_("Translate"), "https://l10n.gnome.org/module/whisp/")
    about.add_link(_("Donate"), "https://tanaybhomia.github.io/Whisp/donate.html")

    is_flatpak = "FLATPAK_ID" in os.environ
    os_name = "Unknown OS"
    try:
        with open("/etc/os-release") as f:
            for line in f:
                if line.startswith("PRETTY_NAME="):
                    os_name = line.split("=")[1].strip().strip('"')
                    break
    except Exception:
        pass

    debug_info = [
        f"os: {os_name}",
        f"prefix: {sys.prefix}",
        f"flatpak: {str(is_flatpak).lower()}",
        f"version: {version}",
        f"python: {platform.python_version()}",
        f"gtk: {Gtk.get_major_version()}.{Gtk.get_minor_version()}.{Gtk.get_micro_version()}",
    ]

    try:
        debug_info.append(f"libadwaita: {Adw.get_major_version()}.{Adw.get_minor_version()}.{Adw.get_micro_version()}")
    except Exception:
        pass

    try:
        import pytesseract  # type: ignore
        debug_info.append(f"tesseract: {pytesseract.get_tesseract_version()}")
    except Exception:
        pass

    try:
        from PIL import Image
        debug_info.append(f"pillow: {Image.__version__}")
    except Exception:
        pass

    about.set_debug_info("\n".join(debug_info))

    if hasattr(about, "set_release_notes"):
        _ver, releases_list = get_latest_release_info(last_seen="0.0.0", only_latest=True, as_list=True)
        if releases_list:
            rel = releases_list[0]
            about.set_release_notes_version(rel.get("version", version))
            raw_xml = rel.get("raw_xml", "")
            if raw_xml:
                about.set_release_notes(raw_xml)

    target_parent = parent_window if isinstance(parent_window, Gtk.Window) else None
    about.present(target_parent)
    if open_page:
        def _find_nav(widget: Gtk.Widget) -> Adw.NavigationView | None:
            if isinstance(widget, Adw.NavigationView):
                return widget
            child = widget.get_first_child()
            while child:
                res = _find_nav(child)
                if res:
                    return res
                child = child.get_next_sibling()
            return None

        nav = _find_nav(about)
        if nav:
            GLib.idle_add(lambda: nav.push_by_tag(open_page) or False)

    return about
