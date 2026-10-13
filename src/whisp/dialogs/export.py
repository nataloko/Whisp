import re
import gettext
from typing import TYPE_CHECKING
from gi.repository import Gtk, Adw, Gio, GLib

if TYPE_CHECKING:
    from whisp.editor import NoteEditor

_ = gettext.gettext


def export_note(
    parent_window: Gtk.Window,
    editor: "NoteEditor",
    toast_overlay: Adw.ToastOverlay | None = None
) -> None:
    """Export the note currently open in editor to a markdown file via Gtk.FileDialog."""
    dialog = Gtk.FileDialog()
    dialog.set_title(_("Export Note"))

    # Determine initial name based on note content
    title = "note"
    start, end = editor.buffer.get_bounds()
    text = editor.buffer.get_text(start, end, False)
    for line in text.split("\n"):
        line = line.strip()
        if line:
            clean_line = re.sub(r"[^a-zA-Z0-9\s]", "", line).strip()
            words = clean_line.split()
            if words:
                title = "-".join(words[:5]).lower()
            break

    dialog.set_initial_name(f"{title}.md")

    filters = Gio.ListStore.new(Gtk.FileFilter)
    md_filter = Gtk.FileFilter()
    md_filter.set_name(_("Markdown File"))
    md_filter.add_pattern("*.md")
    filters.append(md_filter)
    dialog.set_filters(filters)

    def on_save_response(file_dialog: Gtk.FileDialog, result: Gio.AsyncResult) -> None:
        try:
            file = file_dialog.save_finish(result)
            if file:
                b_start, b_end = editor.buffer.get_bounds()
                content = editor.buffer.get_text(b_start, b_end, False).encode("utf-8")
                file.replace_contents(
                    content,
                    None,
                    False,
                    Gio.FileCreateFlags.NONE,
                    None
                )
                if toast_overlay is not None:
                    toast = Adw.Toast.new(_("Note exported successfully"))
                    toast_overlay.add_toast(toast)
        except GLib.Error as e:
            if "dismissed" not in str(e).lower():
                print(f"File save error: {e}")
        except Exception as e:
            print(f"Error writing to file: {e}")

    target_parent = parent_window if isinstance(parent_window, Gtk.Window) else None
    dialog.save(target_parent, None, on_save_response)
