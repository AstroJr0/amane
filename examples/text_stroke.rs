use amane::{App, Center, Color, Column, LayerWindow, Rectangle, Stack, Text, children};

fn main() {
    App::new().window(view).run();
}

fn view() -> LayerWindow {
    // one edge: white letters with a black stroke
    let single = Text::new("stroke")
        .size(64.0)
        .color(Color::WHITE)
        .stroke(6.0, Color::BLACK);

    /*
     * two edges: the same text three times, widest stroke first,
     * each stroke drawn in its own color with letters to match
     */
    let double = Stack::new(children![
        Text::new("double").size(64.0).color(Color::WHITE).stroke(16.0, Color::WHITE),
        Text::new("double").size(64.0).color(Color::BLACK).stroke(9.0, Color::BLACK),
        Text::new("double").size(64.0).color(Color::WHITE),
    ]);

    let words = Column::new(children![single, double]).gap(30.0);

    LayerWindow::new()
        .width(420.0)
        .height(260.0)
        .child(
            Rectangle::new()
                .width(420.0)
                .height(260.0)
                .fill("#e5191c")
                .align_child(Center, Center)
                .child(words),
        )
}
