import os
import sys
import unittest

sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), '..', 'src')))

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")
from gi.repository import Gtk
from whisp.editor import NoteEditor

class TestEditor(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        Gtk.init()

    def test_im_context_attached(self):
        editor = NoteEditor()
        controllers = editor.textview.observe_controllers()
        im_ctrls = []
        for i in range(controllers.get_n_items()):
            c = controllers.get_item(i)
            if isinstance(c, Gtk.EventControllerKey) and c.get_im_context() is not None:
                im_ctrls.append(c)
        
        self.assertGreaterEqual(len(im_ctrls), 2, "Bubble key controller should share textview IMContext")


    def test_toggle_empty_checkbox_removes_prefix(self):
        editor = NoteEditor()
        editor.buffer.set_text("☐ ")
        editor.toggle_checkbox()
        text = editor.buffer.get_text(editor.buffer.get_start_iter(), editor.buffer.get_end_iter(), False)
        self.assertEqual(text, "", "Empty checkbox line should revert to normal empty line on toggle")

    def test_toggle_checkbox_with_text(self):
        editor = NoteEditor()
        editor.buffer.set_text("☐ Buy milk")
        editor.toggle_checkbox()
        text = editor.buffer.get_text(editor.buffer.get_start_iter(), editor.buffer.get_end_iter(), False)
        self.assertEqual(text, "☑ Buy milk", "Checkbox with text should toggle between unchecked and checked")

    def test_toggle_checkbox_cursor_remains_on_same_line(self):
        editor = NoteEditor()
        editor.buffer.set_text("☐ Buy milk\nSecond line")
        iter0 = editor.buffer.get_start_iter()
        editor.buffer.place_cursor(iter0)
        editor.toggle_checkbox()
        insert_iter = editor.buffer.get_iter_at_mark(editor.buffer.get_insert())
        self.assertEqual(insert_iter.get_line(), 0, "Cursor should remain on line 0 after toggling checkbox")


    def test_auto_convert_markdown_checkbox(self):
        editor = NoteEditor()
        editor.buffer.set_text("- [ ] Task")
        text = editor.buffer.get_text(editor.buffer.get_start_iter(), editor.buffer.get_end_iter(), False)
        self.assertEqual(text, "☐ Task", "Markdown - [ ] should automatically convert to unicode ☐ checkbox")

    def test_slate_mode_mouse_motion_with_banner(self):
        from gi.repository import Adw
        app = Adw.Application(application_id="test.slate.app")
        test_self = self
        def on_activate(app):
            from whisp.window import WhispWindow
            win = WhispWindow(application=app)
            win.is_slate_mode = True
            banner = Adw.Banner(title="Test", button_label="Click")
            win.update_banner = banner
            banner.set_revealed(True)
            win.toolbar_view.add_top_bar(banner)
            win.toolbar_view.set_reveal_top_bars(True)
            
            win.on_mouse_motion(None, 0, 60)
            test_self.assertTrue(win.toolbar_view.get_reveal_top_bars(), "Top bars should stay revealed over banner area in Slate Mode")
            
            win.on_mouse_motion(None, 0, 150)
            test_self.assertFalse(win.toolbar_view.get_reveal_top_bars(), "Top bars should hide when cursor moves below top bars area")
            app.quit()
        app.connect("activate", on_activate)
        app.run([])

    def test_is_paste_note(self):
        editor = NoteEditor()
        editor.buffer.set_text("paste")
        self.assertTrue(editor.is_paste_note())

        editor.buffer.set_text(":paste")
        self.assertTrue(editor.is_paste_note())

        editor.buffer.set_text("# paste")
        self.assertTrue(editor.is_paste_note())

        editor.buffer.set_text("## :paste: my note")
        self.assertTrue(editor.is_paste_note())

        editor.buffer.set_text("Some normal note")
        self.assertFalse(editor.is_paste_note())

        editor.buffer.set_text("toothpaste")
        self.assertFalse(editor.is_paste_note())

    def test_paste_keyword_highlighting(self):
        editor = NoteEditor()
        editor.buffer.set_text("paste\nFirst item")
        editor.highlighter.highlight()
        start = editor.buffer.get_start_iter()
        tags = start.get_tags()
        tag_names = [t.get_property("name") for t in tags]
        self.assertIn("paste_keyword", tag_names)

    def test_start_and_stop_autopaste(self):
        editor = NoteEditor()
        self.assertFalse(editor.is_autopaste_active)

        editor.start_autopaste()
        self.assertTrue(editor.is_autopaste_active)
        toast = editor._autopaste_toast
        self.assertIsNotNone(toast)
        assert toast is not None
        self.assertEqual(toast.get_title(), "AutoPaste active")
        self.assertEqual(toast.get_button_label(), "Stop")

        editor.stop_autopaste(silent=True)
        self.assertFalse(editor.is_autopaste_active)
        self.assertIsNone(editor._autopaste_toast)

    def test_handle_paste_return_toggle(self):
        editor = NoteEditor()
        editor.buffer.set_text("paste")
        editor.buffer.place_cursor(editor.buffer.get_end_iter())

        # First return should activate AutoPaste and add a newline
        res = editor.handle_paste_return()
        self.assertTrue(res)
        self.assertTrue(editor.is_autopaste_active)
        text = editor.buffer.get_text(editor.buffer.get_start_iter(), editor.buffer.get_end_iter(), False)
        self.assertEqual(text, "paste\n")

        # Typing paste again on next line and pressing return should disable AutoPaste and remove the command
        end_iter = editor.buffer.get_end_iter()
        editor.buffer.insert(end_iter, "paste")
        editor.buffer.place_cursor(editor.buffer.get_end_iter())

        res2 = editor.handle_paste_return()
        self.assertTrue(res2)
        self.assertFalse(editor.is_autopaste_active)
        text2 = editor.buffer.get_text(editor.buffer.get_start_iter(), editor.buffer.get_end_iter(), False)
        self.assertEqual(text2, "paste\n")

    def test_autopaste_text_read_appends_with_empty_line(self):
        class DummyClipboard:
            def __init__(self, text):
                self.text = text

            def read_text_finish(self, res):
                return self.text

        editor = NoteEditor()
        editor.buffer.set_text("paste\n")
        editor.start_autopaste()

        editor._on_autopaste_text_read(DummyClipboard("First copied line"), None)
        text1 = editor.buffer.get_text(editor.buffer.get_start_iter(), editor.buffer.get_end_iter(), False)
        self.assertEqual(text1, "paste\n\nFirst copied line\n")

        editor._on_autopaste_text_read(DummyClipboard("Second copied line"), None)
        text2 = editor.buffer.get_text(editor.buffer.get_start_iter(), editor.buffer.get_end_iter(), False)
        self.assertEqual(text2, "paste\n\nFirst copied line\n\nSecond copied line\n")

        # Duplicate copy should be ignored
        editor._on_autopaste_text_read(DummyClipboard("Second copied line"), None)
        text3 = editor.buffer.get_text(editor.buffer.get_start_iter(), editor.buffer.get_end_iter(), False)
        self.assertEqual(text3, "paste\n\nFirst copied line\n\nSecond copied line\n")

        editor.stop_autopaste(silent=True)

    def test_autopaste_middle_of_note(self):
        class DummyClipboard:
            def __init__(self, text):
                self.text = text

            def read_text_finish(self, res):
                return self.text

        editor = NoteEditor()
        editor.buffer.set_text("# Notes\nSome thoughts\n\npaste\n\n## Footer\nEnd of note")
        # Place cursor at end of 'paste' line
        paste_iter = editor.buffer.get_iter_at_offset(len("# Notes\nSome thoughts\n\npaste"))
        editor.buffer.place_cursor(paste_iter)

        res = editor.handle_paste_return()
        self.assertTrue(res)
        self.assertTrue(editor.is_autopaste_active)

        editor._on_autopaste_text_read(DummyClipboard("Middle clipped item"), None)
        full_text = editor.buffer.get_text(editor.buffer.get_start_iter(), editor.buffer.get_end_iter(), False)
        self.assertIn("paste\n\nMiddle clipped item", full_text)
        self.assertIn("## Footer\nEnd of note", full_text)

        editor.stop_autopaste(silent=True)

    def test_autopaste_undo(self):
        class DummyClipboard:
            def __init__(self, text):
                self.text = text

            def read_text_finish(self, res):
                return self.text

        editor = NoteEditor()
        editor.buffer.set_text("paste\n")
        editor.start_autopaste()

        editor._on_autopaste_text_read(DummyClipboard("Undoable item"), None)
        self.assertIn("Undoable item", editor.buffer.get_text(editor.buffer.get_start_iter(), editor.buffer.get_end_iter(), False))

        if editor.buffer.get_can_undo():
            editor.buffer.undo()
            self.assertEqual(editor.buffer.get_text(editor.buffer.get_start_iter(), editor.buffer.get_end_iter(), False), "paste\n")

        editor.stop_autopaste(silent=True)


if __name__ == "__main__":
    unittest.main()
