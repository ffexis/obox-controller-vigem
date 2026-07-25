use winres::WindowsResource;

fn main() {
    let mut res = WindowsResource::new();
    res.set_icon("src/boxicons-joystick-filled.ico");
    res.compile().unwrap();
}
