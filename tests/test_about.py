import os
import sys
import unittest
from unittest.mock import MagicMock

sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), '..', 'src')))

import gi
gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")
from gi.repository import Adw
from whisp.dialogs.about import (
    get_latest_release_info,
    get_dynamic_version,
    show_about_dialog,
    open_donate_url
)
from whisp.dialogs.export import export_note
from whisp.editor import NoteEditor


class TestAboutAndExport(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        Adw.init()

    def test_dynamic_version(self):
        ver = get_dynamic_version()
        self.assertIsInstance(ver, str)
        self.assertNotEqual(ver, "Unknown")
        self.assertGreater(len(ver), 0)

    def test_release_info_as_list(self):
        ver, releases = get_latest_release_info(only_latest=True, as_list=True)
        self.assertIsInstance(ver, str)
        self.assertIsInstance(releases, list)
        self.assertGreaterEqual(len(releases), 1)
        self.assertIn("version", releases[0])

    def test_release_info_as_text(self):
        ver, text = get_latest_release_info(only_latest=True, as_list=False)
        self.assertIsInstance(ver, str)
        self.assertIsInstance(text, str)

    def test_show_about_dialog(self):
        about = show_about_dialog(None)
        self.assertIsInstance(about, Adw.AboutDialog)
        self.assertEqual(about.get_developer_name(), "Tanay Bhomia")

    def test_open_donate_url(self):
        # Ensure open_donate_url doesn't raise exceptions
        try:
            open_donate_url(None)
        except Exception as e:
            self.fail(f"open_donate_url raised unexpected exception: {e}")

    def test_export_note_invocation(self):
        editor = NoteEditor()
        editor.buffer.set_text("Export Test Note")
        mock_window = MagicMock()
        mock_toast_overlay = MagicMock()
        
        # Test export_note runs without crashing
        try:
            export_note(mock_window, editor, mock_toast_overlay)
        except Exception as e:
            self.fail(f"export_note raised unexpected exception: {e}")


if __name__ == "__main__":
    unittest.main()
