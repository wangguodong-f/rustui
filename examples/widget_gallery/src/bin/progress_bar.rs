//! `ProgressBar` — a value bound to a property, a readout that follows it and an animation.
//!
//! Run with `cargo run -p widget-gallery --bin progress_bar`.
//!
//! The frame loop does not advance animations by itself: the application owns an
//! [`AnimationDriver`] and ticks it once per frame from `on_update`.

use std::time::Duration;

use rustui::prelude::*;
use widget_gallery::{frame, run, section, status};

fn main() -> rustui::Result<()> {
    run("ProgressBar", build)
}

fn build(app: &mut AppContext<'_>) {
    let window = app.primary_window().expect("the window was registered");
    let status = status();

    // One property drives the bar and everything that reads it.
    let value = SharedProperty::new(0.35f32);
    let bar = ProgressBar::with_value(value.clone());

    let readout = SharedProperty::new(String::from("35%"));
    let caption = Label::with_text(readout.clone()).role(TextRole::Title);
    {
        let (readout, status) = (readout.clone(), status.clone());
        value.changed().connect(move |v| {
            readout.set(format!("{:.0}%", v * 100.0));
            status.set(format!("value = {v:.2}"));
        });
    }

    // The driver owns the animations; the handle is what callbacks register them with.
    let mut driver = AnimationDriver::new(app.ui().redraw().clone());
    let handle = driver.handle();
    app.on_update(move |_wm| {
        driver.tick();
    });

    let zero = Button::new("0%");
    let half = Button::new("50%");
    let full = Button::new("100%");
    for (button, target) in [(&zero, 0.0f32), (&half, 0.5f32), (&full, 1.0f32)] {
        let (value, status) = (value.clone(), status.clone());
        button.clicked().connect(move |_| {
            value.set(target);
            status.set(format!("Jumped to {:.0}%", target * 100.0));
        });
    }

    let animate = Button::new("Animate to 100%").variant(ButtonVariant::Primary);
    {
        let (value, status) = (value.clone(), status.clone());
        animate.clicked().connect(move |_| {
            // The transition captures its start value when it is created, so rewind first.
            value.set(0.0);
            let mut transition =
                Transition::new(value.clone(), 1.0f32, Duration::from_millis(1600));
            transition.with_easing(Easing::InOutCubic);
            handle.add_transition(transition);
            status.set(String::from("Animating to 100%…"));
        });
    }

    let wm = app.wm();
    let mut page = VBox::new()
        .with_spacing(16.0)
        .with_cross_align(CrossAxisAlign::Stretch);

    page.push(wm.insert(Box::new(section("Progress"))));
    page.push(wm.insert(Box::new(bar)));
    page.push(wm.insert(Box::new(caption)));

    let mut controls = HBox::new().with_spacing(12.0);
    controls.push(wm.insert(Box::new(zero)));
    controls.push(wm.insert(Box::new(half)));
    controls.push(wm.insert(Box::new(full)));
    controls.push(wm.insert(Box::new(animate)));
    page.push(wm.insert(Box::new(controls)));

    let page = wm.insert(Box::new(page));
    frame(
        app,
        window,
        "ProgressBar",
        "A SharedProperty<f32> drives the bar, a bound readout and a driver-backed transition.",
        page,
        &status,
    );
}
