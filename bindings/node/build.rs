//! Sets the linker arguments a Node-API addon needs (on macOS, symbols are
//! resolved by the Node.js process that loads the addon).

fn main() {
    napi_build::setup();
}
