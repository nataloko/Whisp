import gettext
from gi.repository import Gtk

_ = gettext.gettext

class ThemeSnippet(Gtk.ToggleButton):
    def __init__(self, theme_id, group=None):
        super().__init__(group=group)
        self.theme_id = theme_id
        self.set_valign(Gtk.Align.CENTER)
        self.set_halign(Gtk.Align.CENTER)
        self.add_css_class("theme-snippet-btn")
        
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
        box.add_css_class("theme-snippet-preview")
        box.add_css_class(f"paper-{theme_id}")
        
        # Add fake text lines to simulate an editor
        for width in [60, 80, 40]:
            line = Gtk.Box()
            line.add_css_class("fake-text-line")
            line.set_size_request(width, 4)
            line.set_halign(Gtk.Align.START)
            box.append(line)
            
        self.set_child(box)
