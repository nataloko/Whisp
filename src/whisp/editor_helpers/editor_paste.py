import re
import gettext
from typing import TYPE_CHECKING
from gi.repository import GLib, Gdk, Adw

if TYPE_CHECKING:
    from gi.repository import Gtk
    from whisp.window import WhispWindow

_ = gettext.gettext


class EditorPasteMixin:
    """Mixin class providing AutoPaste clipboard monitoring for NoteEditor."""
    buffer: "Gtk.TextBuffer"
    textview: "Gtk.TextView"
    window: "WhispWindow | None"

    if TYPE_CHECKING:
        def get_root(self) -> "Gtk.Root | None": ...

    is_autopaste_active: bool = False
    _clipboard_handler_id = None
    _last_pasted_text: str | None = None
    _autopaste_toast: Adw.Toast | None = None
    _autopaste_mark: "Gtk.TextMark | None" = None

    def is_paste_note(self) -> bool:
        """Check if note header indicates an AutoPaste note."""
        start_iter = self.buffer.get_start_iter()
        end_iter = start_iter.copy()
        end_iter.forward_to_line_end()
        first_line = self.buffer.get_text(start_iter, end_iter, False).strip().lower()
        return bool(re.match(r'^(#{1,6}\s*)?:?paste(\s*[:\s].*)?$', first_line))

    def start_autopaste(self):
        """Enable AutoPaste mode and start monitoring the clipboard."""
        if getattr(self, "is_autopaste_active", False):
            return

        self.is_autopaste_active = True
        self._last_pasted_text = None

        if getattr(self, "_autopaste_mark", None) is None:
            insert_mark = self.buffer.get_insert()
            cursor_iter = self.buffer.get_iter_at_mark(insert_mark)
            self._autopaste_mark = self.buffer.create_mark("autopaste_target", cursor_iter, left_gravity=False)

        display = Gdk.Display.get_default()
        if display:
            clipboard = display.get_clipboard()

            # Record current clipboard content so existing text is not immediately pasted
            def on_initial_text(clip, result):
                try:
                    self._last_pasted_text = clip.read_text_finish(result)
                except Exception:
                    self._last_pasted_text = None

            clipboard.read_text_async(None, on_initial_text)

            self._clipboard_handler_id = clipboard.connect("changed", self._on_clipboard_changed)

        self._show_autopaste_banner()

    def stop_autopaste(self, silent: bool = False):
        """Disable AutoPaste mode and disconnect clipboard monitoring."""
        if not getattr(self, "is_autopaste_active", False):
            return

        self.is_autopaste_active = False

        if self._clipboard_handler_id is not None:
            display = Gdk.Display.get_default()
            if display:
                clipboard = display.get_clipboard()
                try:
                    clipboard.disconnect(self._clipboard_handler_id)
                except Exception:
                    pass
            self._clipboard_handler_id = None

        self._last_pasted_text = None

        mark = getattr(self, "_autopaste_mark", None)
        if mark is not None:
            try:
                self.buffer.delete_mark(mark)
            except Exception:
                pass
            self._autopaste_mark = None

        if self._autopaste_toast:
            try:
                self._autopaste_toast.dismiss()
            except Exception:
                pass
            self._autopaste_toast = None

        if not silent:
            toast = Adw.Toast.new(_("AutoPaste disabled"))
            toast.set_timeout(2)
            self._display_toast(toast)

    def _show_autopaste_banner(self):
        """Show persistent toast banner while AutoPaste is active."""
        if self._autopaste_toast:
            try:
                self._autopaste_toast.dismiss()
            except Exception:
                pass

        toast = Adw.Toast.new(_("AutoPaste active"))
        toast.set_timeout(0)
        toast.set_button_label(_("Stop"))
        toast.connect("button-clicked", lambda t: self.stop_autopaste())
        self._autopaste_toast = toast
        self._display_toast(toast)

    def handle_paste_return(self) -> bool:
        """Handle Return key for AutoPaste activation and deactivation."""
        if self.buffer.get_has_selection():
            return False

        insert_mark = self.buffer.get_insert()
        cursor_iter = self.buffer.get_iter_at_mark(insert_mark)

        line_start = cursor_iter.copy()
        line_start.set_line_offset(0)
        line_end = cursor_iter.copy()
        line_end.forward_to_line_end()
        line_text = self.buffer.get_text(line_start, line_end, False).strip()

        if re.match(r'^(#{1,6}\s*)?:{0,2}paste:?$', line_text, re.IGNORECASE):
            if getattr(self, "is_autopaste_active", False):
                self.buffer.delete(line_start, line_end)
                self.stop_autopaste()
                return True
            else:
                self.buffer.insert_at_cursor("\n")
                new_cursor = self.buffer.get_iter_at_mark(insert_mark)
                old_mark = getattr(self, "_autopaste_mark", None)
                if old_mark is not None:
                    self.buffer.delete_mark(old_mark)
                self._autopaste_mark = self.buffer.create_mark("autopaste_target", new_cursor, left_gravity=False)
                self.start_autopaste()
                GLib.idle_add(lambda: self.textview.scroll_mark_onscreen(self.buffer.get_insert()) or False)
                return True
        return False

    def _display_toast(self, toast: Adw.Toast):
        """Helper to show a toast via window's toast_overlay."""
        win = getattr(self, "window", None)
        if win is None and hasattr(self, "get_root"):
            root = self.get_root()
            if root and hasattr(root, "toast_overlay"):
                win = root
        toast_overlay = getattr(win, "toast_overlay", None)
        if toast_overlay is not None:
            toast_overlay.add_toast(toast)

    def _on_clipboard_changed(self, clipboard):
        """Callback triggered when system clipboard content changes."""
        if not getattr(self, "is_autopaste_active", False):
            return
        clipboard.read_text_async(None, self._on_autopaste_text_read)

    def _on_autopaste_text_read(self, clipboard, result):
        """Callback when clipboard text has been read asynchronously."""
        if not getattr(self, "is_autopaste_active", False):
            return

        try:
            text = clipboard.read_text_finish(result)
        except Exception:
            return

        if not text or not text.strip():
            return

        text = text.replace("\r\n", "\n").replace("\r", "\n")

        # Avoid duplicate pasting of the same clipboard copy
        if text == self._last_pasted_text:
            return
        self._last_pasted_text = text

        # Insert on next line separated by an empty line in user action for single Ctrl+Z undo
        self.buffer.begin_user_action()
        try:
            mark = getattr(self, "_autopaste_mark", None)
            if mark is not None and not mark.get_deleted():
                target_iter = self.buffer.get_iter_at_mark(mark)
            else:
                target_iter = self.buffer.get_end_iter()

            # Ensure an empty line before this paste if preceded by text
            if target_iter.get_offset() > 0:
                prev_iter = target_iter.copy()
                prev_iter.backward_char()
                last_char = prev_iter.get_char()
                if last_char != '\n':
                    self.buffer.insert(target_iter, '\n\n')
                else:
                    if prev_iter.get_offset() > 0:
                        prev2_iter = prev_iter.copy()
                        prev2_iter.backward_char()
                        if prev2_iter.get_char() != '\n':
                            self.buffer.insert(target_iter, '\n')
                    else:
                        self.buffer.insert(target_iter, '\n')

            # Re-fetch target_iter after inserting preceding newlines
            if mark is not None and not mark.get_deleted():
                target_iter = self.buffer.get_iter_at_mark(mark)
            else:
                target_iter = self.buffer.get_end_iter()

            insert_text = text.strip('\r\n') + '\n'
            self.buffer.insert(target_iter, insert_text)
        finally:
            self.buffer.end_user_action()

        # Place cursor at the target mark or end, and scroll into view
        mark = getattr(self, "_autopaste_mark", None)
        if mark is not None and not mark.get_deleted():
            end_paste_iter = self.buffer.get_iter_at_mark(mark)
        else:
            end_paste_iter = self.buffer.get_end_iter()
        self.buffer.place_cursor(end_paste_iter)
        GLib.idle_add(lambda: self.textview.scroll_mark_onscreen(self.buffer.get_insert()) or False)
