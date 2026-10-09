//! `Button` — variants, dynamic captions, tooltips and the disabled state.
//!
//! Run with `cargo run -p widget-gallery --bin button`.

use rustui::core::widget::Object;
use rustui::prelude::*;
use widget_gallery::{frame, run, section, status};

fn main() -> rustui::Result<()> {
    run("Button", build)
}

fn build(app: &mut AppContext<'_>) {
    let window = app.primary_window().expect("the window was registered");
    let status = status();

    // The three variants share behaviour and differ only in weight.
    let primary = Button::new("Primary")
        .variant(ButtonVariant::Primary)
        .tooltip("ButtonVariant::Primary — the default");
    let secondary = Button::new("Secondary").variant(ButtonVariant::Secondary);
    let flat = Button::new("Flat").variant(ButtonVariant::Flat);

    // A caption bound to a property: the button repaints whenever the property changes.
    let caption = SharedProperty::new(String::from("Clicks: 0"));
    let counter = Button::with_text(caption.clone());

    // Disabled keeps the widget in the tree; it just refuses interaction.
    let mut disabled = Button::new("Disabled");
    disabled.base_mut().set_enabled(false);

    let clicks = std::cell::Cell::new(0u32);
    {
        let (caption, status) = (caption.clone(), status.clone());
        counter.clicked().connect(move |_| {
            let count = clicks.get() + 1;
            clicks.set(count);
            caption.set(format!("Clicks: {count}"));
            status.set(format!("Pressed the bound button {count} time(s)"));
        });
    }
    for (button, label) in [
        (&primary, "Primary"),
        (&secondary, "Secondary"),
        (&flat, "Flat"),
    ] {
        let status = status.clone();
        button
            .clicked()
            .connect(move |_| status.set(format!("{label} button pressed")));
    }

    let wm = app.wm();

    let mut variants = HBox::new().with_spacing(12.0);
    variants.push(wm.insert(Box::new(primary)));
    variants.push(wm.insert(Box::new(secondary)));
    variants.push(wm.insert(Box::new(flat)));
    let variants = wm.insert(Box::new(variants));

    let mut bound = HBox::new().with_spacing(12.0);
    bound.push(wm.insert(Box::new(counter)));
    bound.push(wm.insert(Box::new(disabled)));
    let bound = wm.insert(Box::new(bound));

    let mut page = VBox::new()
        .with_spacing(16.0)
        .with_cross_align(CrossAxisAlign::Stretch);
    page.push(wm.insert(Box::new(section("Variants"))));
    page.push(variants);
    page.push(wm.insert(Box::new(section("Bound caption and disabled state"))));
    page.push(bound);
    let page = wm.insert(Box::new(page));

    frame(
        app,
        window,
        "Button",
        "Three variants, a caption bound to a SharedProperty, a tooltip and a disabled button.",
        page,
        &status,
    );
}
