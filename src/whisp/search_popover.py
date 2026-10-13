import os
import re
from pathlib import Path
from typing import cast, Any, TYPE_CHECKING
from gi.repository import Gtk, GLib, Pango
from whisp.config import DATA_DIR
from whisp.notes import NoteIndex
from whisp.editor import NoteEditor

if TYPE_CHECKING:
    from whisp.window import WhispWindow


class NoteListBoxRow(Gtk.ListBoxRow):
    file_path: Path | None
    match_index: int | None
    match_term: str

    def __init__(self, **kwargs: Any):
        super().__init__(**kwargs)
        self.file_path = None
        self.match_index = None
        self.match_term = ""


class NoteSearchPopover(Gtk.Popover):
    """In-app search popover with debouncing and lazy-loaded paginated result rows."""
    PAGE_SIZE = 20

    def __init__(self, parent_window: "WhispWindow"):
        super().__init__()
        self.parent_window = parent_window
        self.set_autohide(False)

        popover_box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
        popover_box.set_margin_top(12)
        popover_box.set_margin_bottom(12)
        popover_box.set_margin_start(12)
        popover_box.set_margin_end(12)
        popover_box.set_size_request(320, -1)
        self.set_child(popover_box)

        self.search_entry = Gtk.SearchEntry()
        self.search_timeout_id = 0
        self._row_iter = None
        self.note_index = NoteIndex()
        self.search_entry.connect("search-changed", self.on_search_changed)
        self.search_entry.connect("stop-search", lambda e: self.popdown())
        popover_box.append(self.search_entry)

        scrolled = Gtk.ScrolledWindow()
        scrolled.set_min_content_height(200)
        scrolled.set_min_content_width(296)
        scrolled.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        self.search_scrolled = scrolled
        scrolled.connect("edge-reached", self.on_search_edge_reached)
        scrolled.get_vadjustment().connect("value-changed", self.on_search_scrolled)
        popover_box.append(scrolled)

        self.note_listbox = Gtk.ListBox()
        self.note_listbox.set_selection_mode(Gtk.SelectionMode.SINGLE)
        self.note_listbox.add_css_class("boxed-list")
        self.note_listbox.connect("row-activated", self.on_note_row_activated)
        scrolled.set_child(self.note_listbox)

        self.connect("notify::visible", self.on_popover_visible)

    def on_popover_visible(self, popover: Gtk.Popover, param: Any) -> None:
        if self.get_visible():
            self.search_entry.set_text("")
            self.populate_note_list()
            self.search_entry.grab_focus()
        else:
            if self.search_timeout_id:
                GLib.source_remove(self.search_timeout_id)
                self.search_timeout_id = 0
            for i in range(self.parent_window.carousel.get_n_pages()):
                editor = cast(NoteEditor, self.parent_window.carousel.get_nth_page(i))
                editor.set_search_highlight("")
            editor = self.parent_window.get_current_editor()
            if editor:
                editor.textview.grab_focus()

    def open_search(self, terms: str) -> None:
        """Open the in-app search popover, pre-filled with the given terms."""
        if not self.parent_window.get_mapped():
            def on_map(w: Gtk.Widget) -> None:
                w.disconnect(handler_id)
                self.open_search(terms)
            handler_id = self.parent_window.connect("map", on_map)
            return
        self.parent_window.search_btn.set_active(True)
        self.search_entry.set_text(terms)

    def _build_snippet_markup(self, content: str, idx: int, search_text: str) -> str:
        pre, post = 12, 60
        start = max(0, idx - pre)
        end = min(len(content), idx + len(search_text) + post)
        match_offset = idx - start

        snippet = content[start:end]
        before = re.sub(r"\s+", " ", snippet[:match_offset])
        matched = re.sub(r"\s+", " ", snippet[match_offset:match_offset + len(search_text)])
        after = re.sub(r"\s+", " ", snippet[match_offset + len(search_text):])

        prefix = "…" if start > 0 else ""
        suffix = "…" if end < len(content) else ""

        return (
            GLib.markup_escape_text(prefix + before)
            + '<span background="#f9f06b" color="#000000">'
            + GLib.markup_escape_text(matched)
            + "</span>"
            + GLib.markup_escape_text(after + suffix)
        )

    def _iter_row_descriptors(self, files: list[Path], search_text: str):
        for f in files:
            entry = self.note_index.load(f)
            if entry is None or entry["blank"]:
                continue
            title, tag_str, content = entry["title"], entry["tag_str"], entry["content"]
            if not search_text:
                yield {"file": f, "plain": True, "title": title, "tag_str": tag_str}
                continue
            if search_text not in entry["low_content"]:
                continue
            n = 0
            for idx in self.note_index.iter_body_offsets(entry, search_text):
                yield {
                    "file": f, "content": content, "idx": idx, "occurrence": n,
                    "search": search_text,
                    "title": title if n == 0 else None,
                    "tag_str": tag_str if n == 0 else None,
                }
                n += 1
            if n == 0:
                yield {"file": f, "plain": True, "title": title, "tag_str": tag_str}

    def populate_note_list(self, search_text: str = "") -> None:
        while child := self.note_listbox.get_first_child():
            self.note_listbox.remove(child)

        search_text = search_text.lower()
        files = sorted(DATA_DIR.glob("*.md"), key=lambda f: os.path.getmtime(f) if f.exists() else 0, reverse=True)
        self._row_iter = self._iter_row_descriptors(files, search_text)
        self._render_next_page()

    def _render_next_page(self) -> None:
        if self._row_iter is None:
            return
        count = 0
        for d in self._row_iter:
            self.note_listbox.append(self._row_from_descriptor(d))
            count += 1
            if count >= self.PAGE_SIZE:
                return
        self._row_iter = None

    def _row_from_descriptor(self, d: dict[str, Any]) -> NoteListBoxRow:
        if d.get("plain"):
            row = self._make_note_row(d["file"])
            self._append_header(row.get_child(), d["title"], d["tag_str"])
            return row
        return self._make_snippet_row(
            d["file"], d["content"], d["idx"], d["occurrence"], d["search"],
            title=d["title"], tag_str=d["tag_str"],
        )

    def _append_header(self, vbox: Gtk.Widget | None, title: str, tag_str: str | None) -> None:
        if not isinstance(vbox, Gtk.Box):
            return
        title_label = Gtk.Label(label=title, xalign=0)
        title_label.set_ellipsize(Pango.EllipsizeMode.END)
        title_label.set_max_width_chars(32)
        vbox.append(title_label)
        if tag_str:
            tag_label = Gtk.Label(label=tag_str, xalign=0)
            tag_label.add_css_class("dim-label")
            tag_label.set_ellipsize(Pango.EllipsizeMode.END)
            tag_label.set_max_width_chars(32)
            vbox.append(tag_label)

    def _make_note_row(self, file_path: Path, indent: bool = False) -> NoteListBoxRow:
        row = NoteListBoxRow()
        vbox = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        vbox.set_margin_start(28 if indent else 12)
        vbox.set_margin_end(12)
        vbox.set_margin_top(8)
        vbox.set_margin_bottom(8)
        row.set_child(vbox)
        row.file_path = file_path
        row.match_index = None
        row.match_term = ""
        return row

    def _make_snippet_row(
        self,
        f: Path,
        content: str,
        idx: int,
        occurrence_index: int,
        search_text: str,
        title: str | None = None,
        tag_str: str | None = None
    ) -> NoteListBoxRow:
        row = self._make_note_row(f, indent=title is None)
        vbox = cast(Gtk.Box, row.get_child())
        if vbox:
            if title is not None:
                self._append_header(vbox, title, tag_str)

            snippet_label = Gtk.Label(xalign=0)
            snippet_label.set_markup(self._build_snippet_markup(content, idx, search_text))
            snippet_label.add_css_class("dim-label")
            snippet_label.set_ellipsize(Pango.EllipsizeMode.END)
            snippet_label.set_max_width_chars(32)
            snippet_label.set_wrap(False)
            vbox.append(snippet_label)

        row.match_index = occurrence_index
        row.match_term = search_text
        return row

    def on_search_edge_reached(self, scrolled: Gtk.ScrolledWindow, pos: Gtk.PositionType) -> None:
        if pos == Gtk.PositionType.BOTTOM:
            self._render_next_page()

    def on_search_scrolled(self, vadj: Gtk.Adjustment) -> None:
        if self._row_iter is None:
            return
        remaining_below = vadj.get_upper() - (vadj.get_value() + vadj.get_page_size())
        if remaining_below < vadj.get_page_size():
            self._render_next_page()

    def on_search_changed(self, entry: Gtk.SearchEntry) -> None:
        if self.search_timeout_id:
            GLib.source_remove(self.search_timeout_id)
        self.search_timeout_id = GLib.timeout_add(150, self._run_search, entry.get_text())

    def _run_search(self, text: str) -> bool:
        self.search_timeout_id = 0
        self.populate_note_list(text)
        editor = self.parent_window.get_current_editor()
        if editor:
            editor.set_search_highlight(text)
        return False

    def on_note_row_activated(self, listbox: Gtk.ListBox, row: Gtk.ListBoxRow) -> None:
        file_path = getattr(row, "file_path", None)
        if file_path:
            match_index = getattr(row, "match_index", None)
            match_term = getattr(row, "match_term", "")
            self.parent_window.open_note_at_match(file_path, match_term, match_index)
        self.popdown()
