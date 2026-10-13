import os
import sys
import unittest
from pathlib import Path
from unittest.mock import MagicMock

sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), '..', 'src')))

import gi
gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")
from gi.repository import Gtk, Adw
from whisp.search_popover import NoteSearchPopover, NoteListBoxRow


class TestSearchPopover(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        Adw.init()

    def setUp(self):
        self.mock_window = MagicMock()
        self.mock_window.carousel.get_n_pages.return_value = 0
        self.popover = NoteSearchPopover(self.mock_window)

    def test_note_list_box_row(self):
        row = NoteListBoxRow()
        self.assertIsInstance(row, Gtk.ListBoxRow)
        self.assertIsNone(row.file_path)
        self.assertIsNone(row.match_index)
        self.assertEqual(row.match_term, "")

    def test_search_popover_init(self):
        self.assertIsInstance(self.popover, Gtk.Popover)
        self.assertIsInstance(self.popover.search_entry, Gtk.SearchEntry)
        self.assertIsInstance(self.popover.note_listbox, Gtk.ListBox)
        self.assertFalse(self.popover.get_autohide())

    def test_build_snippet_markup(self):
        markup = self.popover._build_snippet_markup("Buy milk at grocery", 4, "milk")
        self.assertIn('<span background="#f9f06b" color="#000000">milk</span>', markup)
        self.assertIn("Buy", markup)

    def test_make_note_row(self):
        row = self.popover._make_note_row(Path("test.md"))
        self.assertIsInstance(row, NoteListBoxRow)
        self.assertEqual(row.file_path, Path("test.md"))

    def test_row_activated(self):
        row = NoteListBoxRow()
        row.file_path = Path("sample.md")
        row.match_index = 2
        row.match_term = "sample"
        self.popover.on_note_row_activated(self.popover.note_listbox, row)
        self.mock_window.open_note_at_match.assert_called_once_with(Path("sample.md"), "sample", 2)


if __name__ == "__main__":
    unittest.main()
