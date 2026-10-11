import re
from typing import TYPE_CHECKING
from gi.repository import GLib

if TYPE_CHECKING:
    from gi.repository import Gtk

class EditorListMixin:
    """Mixin class containing list management, checkbox toggling, and line movement operations for NoteEditor."""
    buffer: "Gtk.TextBuffer"
    textview: "Gtk.TextView"

    
    def convert_markdown_checkboxes(self):
        if getattr(self, "_converting_checkboxes", False):
            return
        
        start_iter = self.buffer.get_start_iter()
        end_iter = self.buffer.get_end_iter()
        text = self.buffer.get_text(start_iter, end_iter, False)
        
        m_empty = list(re.finditer(r"^(\s*)([-*+])\s*\[\s*\](\s?)", text, re.MULTILINE))
        m_checked = list(re.finditer(r"^(\s*)([-*+])\s*\[[xX]\](\s?)", text, re.MULTILINE))
        
        if not m_empty and not m_checked:
            return

        self._converting_checkboxes = True
        try:
            all_matches = []
            for m in m_empty:
                all_matches.append((m.start(), m.end(), m.group(1) + "☐ "))
            for m in m_checked:
                all_matches.append((m.start(), m.end(), m.group(1) + "☑ "))
                
            all_matches.sort(key=lambda x: x[0], reverse=True)
            
            for start_off, end_off, new_str in all_matches:
                s_mark = self.buffer.create_mark(None, self.buffer.get_iter_at_offset(start_off), True)
                e_mark = self.buffer.create_mark(None, self.buffer.get_iter_at_offset(end_off), False)
                self.buffer.delete(self.buffer.get_iter_at_mark(s_mark), self.buffer.get_iter_at_mark(e_mark))
                self.buffer.insert(self.buffer.get_iter_at_mark(s_mark), new_str)
                self.buffer.delete_mark(s_mark)
                self.buffer.delete_mark(e_mark)
        finally:
            self._converting_checkboxes = False

    def get_line_text(self, line_num: int) -> str:
        if line_num < 0 or line_num >= self.buffer.get_line_count():
            return ""
        _, start = self.buffer.get_iter_at_line(line_num)
        end = start.copy()
        if not end.ends_line():
            end.forward_to_line_end()
        return self.buffer.get_text(start, end, False)

    def move_line(self, direction: int) -> bool:
        """Moves current line (and its subtree if it's a list item) up (-1) or down (1)."""
        insert_mark = self.buffer.get_insert()
        cursor_iter = self.buffer.get_iter_at_mark(insert_mark)
        
        curr_line = cursor_iter.get_line()
        curr_text = self.get_line_text(curr_line)
        
        list_regex = r'^(\s*)([-*+]\s+|\d+\.\s+|- \[ \]\s+|[☐☑]\s*)'
        curr_match = re.match(list_regex, curr_text)
        if not curr_match:
            # Normal single-line swap
            target_line = curr_line + direction
            if target_line < 0 or target_line >= self.buffer.get_line_count():
                return False
            upper_start = min(curr_line, target_line)
            upper_end = upper_start
            lower_start = max(curr_line, target_line)
            lower_end = lower_start
        else:
            # Tree-aware swap
            curr_indent = len(curr_match.group(1))
            subtree_start = curr_line
            subtree_end = curr_line
            
            # Find end of current subtree
            for i in range(curr_line + 1, self.buffer.get_line_count()):
                text = self.get_line_text(i)
                if not text.strip():
                    break
                m = re.match(r'^(\s*)', text)
                indent = len(m.group(1)) if m else 0
                if indent > curr_indent:
                    subtree_end = i
                else:
                    break
                    
            sibling_start = None
            sibling_end = None
            
            if direction == -1:
                # Look up for previous sibling
                for i in range(curr_line - 1, -1, -1):
                    text = self.get_line_text(i)
                    if not text.strip():
                        break
                    m = re.match(r'^(\s*)', text)
                    indent = len(m.group(1)) if m else 0
                    if indent < curr_indent:
                        break # hit parent
                    if indent == curr_indent and re.match(list_regex, text):
                        sibling_start = i
                        # Find end of previous sibling subtree
                        sibling_end = i
                        for j in range(i + 1, curr_line):
                            t = self.get_line_text(j)
                            if not t.strip():
                                break
                            m2 = re.match(r'^(\s*)', t)
                            ind = len(m2.group(1)) if m2 else 0
                            if ind > curr_indent:
                                sibling_end = j
                            else:
                                break
                        break
                        
                if sibling_start is None:
                    return False
                upper_start = sibling_start
                upper_end = sibling_end
                lower_start = subtree_start
                lower_end = subtree_end
            else:
                # Look down for next sibling
                next_check = subtree_end + 1
                if next_check < self.buffer.get_line_count():
                    text = self.get_line_text(next_check)
                    m = re.match(r'^(\s*)', text)
                    indent = len(m.group(1)) if m else 0
                    if indent == curr_indent and re.match(list_regex, text):
                        sibling_start = next_check
                        sibling_end = next_check
                        for j in range(next_check + 1, self.buffer.get_line_count()):
                            t = self.get_line_text(j)
                            if not t.strip():
                                break
                            m2 = re.match(r'^(\s*)', t)
                            ind = len(m2.group(1)) if m2 else 0
                            if ind > curr_indent:
                                sibling_end = j
                            else:
                                break
                if sibling_start is None or sibling_end is None:
                    return False
                upper_start = subtree_start
                upper_end = subtree_end
                lower_start = sibling_start
                lower_end = sibling_end

        if upper_start is None or upper_end is None or lower_start is None or lower_end is None:
            return False

        # Perform block swap
        self.buffer.begin_user_action()
        try:
            _, u_s = self.buffer.get_iter_at_line(upper_start)
            _, u_e = self.buffer.get_iter_at_line(upper_end)
            if not u_e.ends_line():
                u_e.forward_to_line_end()
            upper_text = self.buffer.get_text(u_s, u_e, False)

            _, l_s = self.buffer.get_iter_at_line(lower_start)
            _, l_e = self.buffer.get_iter_at_line(lower_end)
            if not l_e.ends_line():
                l_e.forward_to_line_end()
            lower_text = self.buffer.get_text(l_s, l_e, False)

            # Replace lower block first, then upper block
            s_l_mark = self.buffer.create_mark(None, l_s, True)
            e_l_mark = self.buffer.create_mark(None, l_e, False)
            self.buffer.delete(l_s, l_e)
            self.buffer.insert(self.buffer.get_iter_at_mark(s_l_mark), upper_text)

            s_u_mark = self.buffer.create_mark(None, u_s, True)
            e_u_mark = self.buffer.create_mark(None, u_e, False)
            self.buffer.delete(u_s, u_e)
            self.buffer.insert(self.buffer.get_iter_at_mark(s_u_mark), lower_text)

            # Place cursor at start of moved line/subtree
            new_line = lower_start if direction == -1 else upper_start + (lower_end - lower_start + 1)
            _, new_iter = self.buffer.get_iter_at_line(new_line)
            self.buffer.place_cursor(new_iter)
        finally:
            self.buffer.end_user_action()
        return True

    def handle_tab(self):
        insert_mark = self.buffer.get_insert()
        cursor_iter = self.buffer.get_iter_at_mark(insert_mark)
        line_start = cursor_iter.copy()
        line_start.set_line_offset(0)
        
        # Get current line text to find its indent
        line_end = cursor_iter.copy()
        line_end.forward_to_line_end()
        current_text = self.buffer.get_text(line_start, line_end, False)
        m_curr = re.match(r'^(\s*)', current_text)
        curr_indent_len = len(m_curr.group(1)) if m_curr else 0
        
        # Check previous line
        prev_line_iter = line_start.copy()
        if not prev_line_iter.backward_line():
            return True # First line, no indentation allowed
            
        prev_line_end = prev_line_iter.copy()
        prev_line_end.forward_to_line_end()
        prev_text = self.buffer.get_text(prev_line_iter, prev_line_end, False)
        
        m_prev = re.match(r'^(\s*)', prev_text)
        prev_indent_len = len(m_prev.group(1)) if m_prev else 0
        
        if curr_indent_len >= prev_indent_len + 4:
            return True # Already indented 1 level deeper than the previous line
            
        self.buffer.insert(line_start, "    ")
        self.recalculate_list_number(line_start)
        return True

    def handle_shift_tab(self):
        insert_mark = self.buffer.get_insert()
        cursor_iter = self.buffer.get_iter_at_mark(insert_mark)
        line_start = cursor_iter.copy()
        line_start.set_line_offset(0)
        
        check_iter = line_start.copy()
        check_iter.forward_chars(4)
        text = self.buffer.get_text(line_start, check_iter, False)
        
        spaces_to_remove = 0
        for char in text:
            if char == ' ':
                spaces_to_remove += 1
            else:
                break
                
        if spaces_to_remove > 0:
            del_iter = line_start.copy()
            del_iter.forward_chars(spaces_to_remove)
            self.buffer.delete(line_start, del_iter)
            self.recalculate_list_number(line_start)
            return True
        return False

    def recalculate_list_number(self, line_start_iter):
        line_start = line_start_iter.copy()
        line_start.set_line_offset(0)
        line_end = line_start.copy()
        line_end.forward_to_line_end()
        text = self.buffer.get_text(line_start, line_end, False)
        
        m_ol = re.match(r'^(\s*)(\d+)\.(\s+.*)$', text)
        if not m_ol:
            return
            
        current_indent_len = len(m_ol.group(1))
        
        search_iter = line_start.copy()
        prev_num = 0
        while search_iter.backward_line():
            s_end = search_iter.copy()
            s_end.forward_to_line_end()
            s_text = self.buffer.get_text(search_iter, s_end, False)
            
            if not s_text.strip():
                continue
            
            m_search = re.match(r'^(\s*)(\d+)\.\s+.*$', s_text)
            if m_search:
                s_indent_len = len(m_search.group(1))
                if s_indent_len == current_indent_len:
                    prev_num = int(m_search.group(2))
                    break
                elif s_indent_len < current_indent_len:
                    break
            else:
                m_other = re.match(r'^(\s*)', s_text)
                if m_other and len(m_other.group(1)) < current_indent_len:
                    break
                    
        new_num = prev_num + 1
        current_num_str = m_ol.group(2)
        new_num_str = str(new_num)
        
        if current_num_str != new_num_str:
            num_start = line_start.copy()
            num_start.forward_chars(current_indent_len)
            num_end = num_start.copy()
            num_end.forward_chars(len(current_num_str))
            
            self.buffer.delete(num_start, num_end)
            self.buffer.insert(num_start, new_num_str)

    def toggle_checkbox(self):
        insert_mark = self.buffer.get_insert()
        cursor_iter = self.buffer.get_iter_at_mark(insert_mark)
        line_start = cursor_iter.copy()
        line_start.set_line_offset(0)
        line_end = cursor_iter.copy()
        if not line_end.ends_line():
            line_end.forward_to_line_end()
        line_text = self.buffer.get_text(line_start, line_end, False)

        m_indent = re.match(r'^(\s*)', line_text)
        indent = m_indent.group(1) if m_indent else ""
        rest = line_text[len(indent):]

        has_box_or_bullet = bool(re.match(r'^(?:[☐☑]|[-*+]\s*\[[ xX]\]|[-*+])(?:\s+|$)', rest))
        has_checked = bool(re.search(r'(?:☑|\[[xX]\])', rest))

        clean_content = rest
        while True:
            stripped = re.sub(r'^(?:[☐☑]|[-*+]\s*\[[ xX]\]|[-*+])\s*', '', clean_content)
            if stripped == clean_content:
                break
            clean_content = stripped

        if not clean_content.strip():
            if has_box_or_bullet:
                new_line_text = indent
            else:
                new_line_text = f"{indent}☐ "
        else:
            if has_checked:
                new_line_text = f"{indent}☐ {clean_content}"
            elif has_box_or_bullet:
                new_line_text = f"{indent}☑ {clean_content}"
            else:
                new_line_text = f"{indent}☐ {clean_content}"

        cursor_offset = cursor_iter.get_line_offset()
        diff = len(new_line_text) - len(line_text)

        self.buffer.delete(line_start, line_end)
        self.buffer.insert(line_start, new_line_text)

        target_offset = min(len(new_line_text), max(0, cursor_offset + diff))
        new_cursor = line_start.copy()
        new_cursor.set_line_offset(target_offset)
        self.buffer.place_cursor(new_cursor)
        return True

    def count_checkboxes(self):
        start, end = self.buffer.get_bounds()
        text = self.buffer.get_text(start, end, False)
        return len(re.findall(r'^(\s*)[☐☑]', text, re.MULTILINE))

    def is_empty(self):
        start, end = self.buffer.get_bounds()
        text = self.buffer.get_text(start, end, False).strip()
        return len(text) == 0

    def is_list_note(self):
        start_iter = self.buffer.get_start_iter()
        end_iter = start_iter.copy()
        end_iter.forward_to_line_end()
        first_line = self.buffer.get_text(start_iter, end_iter, False).strip().lower()
        return bool(re.match(r'^(#{1,6}\s*)?list(\s*[:\s].*)?$', first_line))

    def handle_return(self):
        insert_mark = self.buffer.get_insert()
        cursor_iter = self.buffer.get_iter_at_mark(insert_mark)
        
        line_start = cursor_iter.copy()
        line_start.set_line_offset(0)
        line_text = self.buffer.get_text(line_start, cursor_iter, False)
        
        is_list = self.is_list_note()

        def insert_sync(text):
            self.buffer.insert_at_cursor(text)
            GLib.idle_add(lambda: self.textview.scroll_mark_onscreen(self.buffer.get_insert()) or False)
            return True

        if is_list:
            m_empty = re.match(r'^(\s*)([☐☑]|[-*+]\s*\[[ xX]\])\s*$', line_text)
            if m_empty:
                self.buffer.delete(line_start, cursor_iter)
                return insert_sync("\n")
                
            m_indent = re.match(r'^(\s*)', line_text)
            indent = m_indent.group(1) if m_indent else ""
            return insert_sync(f"\n{indent}☐ ")

        m_empty = re.match(r'^(\s*)([☐☑]|[-*+]\s*\[[ xX]\])\s*$', line_text)
        if m_empty:
            self.buffer.delete(line_start, cursor_iter)
            return insert_sync("\n")
            
        m_box = re.match(r'^(\s*)([☐☑]|[-*+]\s*\[[ xX]\])\s+(.*)$', line_text)
        if m_box:
            indent, _, content = m_box.groups()
            return insert_sync(f"\n{indent}☐ ")
        
        m_ul = re.match(r'^(\s*)([-*+])\s+(.*)$', line_text)
        if m_ul:
            indent, bullet, content = m_ul.groups()
            if not content.strip():
                self.buffer.delete(line_start, cursor_iter)
                return insert_sync("\n")
            else:
                return insert_sync(f"\n{indent}{bullet} ")
                
        m_ol = re.match(r'^(\s*)(\d+)\.\s+(.*)$', line_text)
        if m_ol:
            indent, num, content = m_ol.groups()
            if not content.strip():
                self.buffer.delete(line_start, cursor_iter)
                return insert_sync("\n")
            else:
                next_num = int(num) + 1
                return insert_sync(f"\n{indent}{next_num}. ")
                
        return False
