//! Finding bar widgets in the GTK tree.

use gtk4 as gtk;
use gtk4::prelude::*;

fn walk(w: &gtk::Widget, f: &mut dyn FnMut(&gtk::Widget)) {
    f(w);
    let mut c = w.first_child();
    while let Some(x) = c {
        walk(&x, f);
        c = x.next_sibling();
    }
}

/// The first caption whose text equals `text`.
pub fn label_with_text(root: &gtk::Widget, text: &str) -> Option<gtk::Label> {
    let mut found = None;
    walk(root, &mut |w| {
        if found.is_none()
            && let Ok(l) = w.clone().downcast::<gtk::Label>()
            && l.text() == text
        {
            found = Some(l);
        }
    });
    found
}

/// All captions with a CSS class.
pub fn labels_with_class(root: &gtk::Widget, class: &str) -> Vec<gtk::Label> {
    let mut out = Vec::new();
    walk(root, &mut |w| {
        if let Ok(l) = w.clone().downcast::<gtk::Label>()
            && l.has_css_class(class)
        {
            out.push(l);
        }
    });
    out
}
