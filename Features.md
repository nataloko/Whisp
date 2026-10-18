# Whisp Feature Roadmap

This document outlines the development roadmap for Whisp, segmented into phases based on priority and release goals.

## Phase 1: Pre-Flathub Polish (Quick Wins)
These features are designed to be high-impact, immediate improvements that finalize the core experience before publishing version 1.0 to Flathub. We will complete this phase, package the Flatpak, wait for the official icon from GNOME Designers, and publish.

- [-x] **Paper Themes (Backgrounds)**
  - Add native CSS styling for `Dotted`, `Grid`, and `Blank` backgrounds to mimic physical engineering paper or scratchpads.
  - Implement a "Paper Style" dropdown in the Preferences window to toggle between themes.
- [-x] **Paste Plain Text**
  - Intercept `Ctrl+Shift+V`.
  - Extract raw text from the GNOME Clipboard and insert it into the editor, actively stripping out any rich-text formatting.
- [-x] **Link Shortener**
  - Implement a shortcut (e.g., `Ctrl+Shift+L`) or a small UI button.
  - Quietly replace long URLs with shortened versions via a free, no-authentication API like `is.gd`.

---

## Phase 2: The Math Engine
This is the core "wow" feature. It transforms Whisp from a static markdown editor into a dynamic, Soulver-style contextual calculator.

- [ ] **Reactive Variables**
  - Build a parser to scan the text line-by-line and identify variable assignments (e.g., `friends: 50`).
  - Store these variables securely in a local evaluator dictionary.
- [ ] **Contextual Evaluator**
  - Safely evaluate arithmetic expressions that reference the stored variables (e.g., `friends * 2`).
- [ ] **Ghost Text UI Overlay**
  - Utilize `Gtk.TextTag` to render the evaluation results (e.g., the green `= 100`) dynamically on the right margin of the editor.
  - Ensure the ghost text is non-selectable and does not pollute the actual saved `.md` file.

---

## Phase 3: The Power User Workflows
These are advanced features relying on background tasks and system-level integrations.

- [-x] **Screenshot to Text (OCR)**
  - Integrate `tesseract-ocr` into the background.
  - Intercept image paste events (`Ctrl+V` when an image is in the clipboard).
  - Run the image through the OCR engine locally and dump the extracted text directly into the scratchpad.
- [-x] **Auto Paste Mode**
  - Create a background listener attached to `Gdk.Clipboard`'s `changed` signal.
  - When enabled, automatically capture any text copied anywhere on the system and append/prepend it to the active Whisp note without requiring window focus.
