//! Accessibility: name and role of an element through one helper.
//!
//! Skeleton rule: every column and the bar has a landmark role and a name, every button without text has a name.
//! Names come from `en.ftl` through `tr`; the helper receives an already prepared string.

use gtk4 as gtk;
use gtk4::prelude::*;

/// Assign the element an accessible name (the `label` property).
pub fn set_name(widget: &impl IsA<gtk::Accessible>, name: &str) {
    widget.update_property(&[gtk::accessible::Property::Label(name)]);
}

/// Container with a landmark role and a name. The role is set at creation (GTK does not allow changing it later).
pub fn landmark(role: gtk::AccessibleRole, orientation: gtk::Orientation, name: &str) -> gtk::Box {
    let container = gtk::Box::builder()
        .orientation(orientation)
        .accessible_role(role)
        .build();
    set_name(&container, name);
    container
}

/// Column landmark: a vertical container with the `Region` role.
pub fn region(name: &str) -> gtk::Box {
    landmark(
        gtk::AccessibleRole::Region,
        gtk::Orientation::Vertical,
        name,
    )
}

/// Icon button: a name for the screen reader and the same caption in the tooltip.
pub fn icon_button(icon: &str, name: &str) -> gtk::Button {
    let button = gtk::Button::from_icon_name(icon);
    set_name(&button, name);
    button.set_tooltip_text(Some(name));
    button
}
