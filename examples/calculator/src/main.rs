//! A four function calculator: `Grid` layout, `Button` signals and application state.
//!
//! Run it with `cargo run -p calculator`. The arithmetic lives in [`Calc`], a plain struct owned
//! by the application; the widgets only report key presses and show `Calc`'s `SharedProperty`,
//! which is the Model / View split in miniature.

use std::cell::RefCell;
use std::rc::Rc;

use rustui::prelude::*;

fn main() -> rustui::Result<()> {
    Application::new()?
        .window(
            WindowSpec::new()
                .title("RustUI — Calculator")
                .size(320.0, 440.0)
                .min_size(260.0, 360.0),
        )
        .run(build)
}

/// Key legend and the code each button sends to [`Calc`].
const KEYS: [[(&str, char); 4]; 5] = [
    [("C", 'C'), ("±", 'n'), ("%", '%'), ("÷", '/')],
    [("7", '7'), ("8", '8'), ("9", '9'), ("×", '*')],
    [("4", '4'), ("5", '5'), ("6", '6'), ("−", '-')],
    [("1", '1'), ("2", '2'), ("3", '3'), ("+", '+')],
    [("0", '0'), (".", '.'), ("=", '='), ("⌫", 'B')],
];

/// The whole state of the calculator; nothing here knows about widgets.
#[derive(Default)]
struct Calc {
    /// What the display shows.
    entry: String,
    /// Left hand side of the pending operation.
    acc: f64,
    /// Pending operation, if any.
    op: Option<char>,
    /// The next digit starts a fresh entry instead of appending.
    fresh: bool,
    /// Bound to the display label.
    display: SharedProperty<String>,
}

impl Calc {
    /// Handles one key press and refreshes the display.
    fn press(&mut self, code: char) {
        match code {
            '0'..='9' => self.digit(code),
            '.' => self.dot(),
            '+' | '-' | '*' | '/' => self.operator(code),
            '=' => self.equals(),
            'C' => self.clear(),
            '%' => self.percent(),
            'n' => self.negate(),
            'B' => self.backspace(),
            _ => {}
        }
        let text = if self.entry.is_empty() {
            String::from("0")
        } else {
            self.entry.clone()
        };
        self.display.set(text);
    }

    fn digit(&mut self, digit: char) {
        if self.fresh {
            self.entry.clear();
            self.fresh = false;
        }
        if self.entry.len() >= 16 {
            return;
        }
        if self.entry == "0" {
            self.entry.clear();
        }
        self.entry.push(digit);
    }

    fn dot(&mut self) {
        if self.fresh {
            self.entry.clear();
            self.fresh = false;
        }
        if self.entry.is_empty() {
            self.entry.push_str("0.");
        } else if !self.entry.contains('.') {
            self.entry.push('.');
        }
    }

    fn backspace(&mut self) {
        if self.fresh {
            return;
        }
        self.entry.pop();
    }

    fn operator(&mut self, op: char) {
        // A pending operation is evaluated as soon as the next one is chosen.
        if self.op.is_some() && !self.fresh {
            self.evaluate();
        } else if self.entry.is_empty() || self.fresh {
            self.entry = format_number(self.acc);
        }
        self.acc = self.value();
        self.op = Some(op);
        self.fresh = true;
    }

    fn equals(&mut self) {
        if self.op.is_some() {
            self.evaluate();
        }
        self.op = None;
        self.fresh = true;
    }

    fn evaluate(&mut self) {
        let left = self.acc;
        let right = self.value();
        let result = match self.op {
            Some('+') => left + right,
            Some('-') => left - right,
            Some('*') => left * right,
            Some('/') => left / right,
            _ => right,
        };
        self.entry = format_number(result);
        self.acc = result;
        self.fresh = true;
    }

    fn percent(&mut self) {
        self.entry = format_number(self.value() / 100.0);
        self.fresh = true;
    }

    fn negate(&mut self) {
        if let Some(rest) = self.entry.strip_prefix('-') {
            self.entry = rest.to_string();
        } else if !self.entry.is_empty() {
            self.entry.insert(0, '-');
        } else if !self.fresh {
            self.entry.push('-');
        }
    }

    fn clear(&mut self) {
        self.entry = String::new();
        self.acc = 0.0;
        self.op = None;
        self.fresh = false;
    }

    fn value(&self) -> f64 {
        self.entry.parse::<f64>().unwrap_or(0.0)
    }
}

/// Renders a value the way a calculator would; `NaN` (a division by zero) reads as `Error`.
fn format_number(value: f64) -> String {
    if !value.is_finite() {
        return String::from("Error");
    }
    let text = format!("{value:.10}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn build(app: &mut AppContext<'_>) {
    let window = app.primary_window().expect("the window was registered");
    let theme = app.ui().theme().clone();

    let display = SharedProperty::new(String::from("0"));
    let calc = Rc::new(RefCell::new(Calc {
        display: display.clone(),
        ..Calc::default()
    }));

    let label = Label::with_text(display)
        .align(TextAlign::Right)
        .role(TextRole::Title);

    let wm = app.wm();
    // The container wraps the label, so the label becomes an object first.
    let label = wm.insert(Box::new(label));
    let screen = Container::new()
        .child(label)
        .with_background(theme.colors.surface)
        .with_border(theme.colors.border, theme.borders.thin)
        .with_radius(theme.radius.md)
        .with_padding(Insets::all(theme.spacing.md))
        .with_fill();
    let screen = wm.insert(Box::new(screen));

    let mut grid = Grid::new()
        .with_columns(vec![Track::fr(1.0); 4])
        .with_rows(vec![Track::fr(1.0); 5])
        .with_spacing(theme.spacing.sm)
        .with_fill();
    for (row, keys) in KEYS.iter().enumerate() {
        for (column, (label, code)) in keys.iter().enumerate() {
            let id = key_button(wm, &calc, label, *code);
            grid.place_child(id, row, column);
        }
    }
    let grid = wm.insert(Box::new(grid));

    let root = VBox::new()
        .with_spacing(theme.spacing.md)
        .with_padding(Insets::all(theme.spacing.md))
        .with_main_axis_size(MainAxisSize::Fill)
        .with_fill()
        .with_child(screen)
        .with_child(grid);
    let root = wm.insert(Box::new(root));
    wm.set_root_id(window, root);
}

/// One button: it only knows which key it presses.
fn key_button(
    wm: &mut WindowManager,
    calc: &Rc<RefCell<Calc>>,
    label: &str,
    code: char,
) -> ObjectId {
    let button = Button::new(label).variant(variant_for(code));
    let calc = Rc::clone(calc);
    button.clicked().connect(move |_| {
        calc.borrow_mut().press(code);
    });
    wm.insert(Box::new(button))
}

fn variant_for(code: char) -> ButtonVariant {
    match code {
        '+' | '-' | '*' | '/' | '=' => ButtonVariant::Primary,
        'C' | 'B' => ButtonVariant::Secondary,
        _ => ButtonVariant::Flat,
    }
}
