use std::{env, fs, path::PathBuf};

fn main() {
    let mut args = env::args_os().skip(1);
    let operation = args.next().expect("dialog operation");
    let expected = PathBuf::from(args.next().expect("fixture path"));
    let directory = expected.parent().expect("fixture directory");
    let name = expected
        .file_name()
        .expect("fixture name")
        .to_str()
        .unwrap();
    let dialog = rfd::FileDialog::new()
        .set_title("Voice App dialog QA")
        .set_directory(directory)
        .add_filter("Preset JSON", &["json"]);

    match operation.to_str().unwrap() {
        "open" => match dialog.pick_file() {
            Some(path) => {
                assert_eq!(path, expected);
                assert_eq!(fs::read_to_string(path).unwrap(), "fixture");
                println!("opened");
            }
            None => println!("cancel"),
        },
        "save" => match dialog.set_file_name(name).save_file() {
            Some(path) => {
                assert_eq!(path, expected);
                fs::write(path, "saved fixture").unwrap();
                println!("saved");
            }
            None => println!("cancel"),
        },
        "message" => {
            let result = rfd::MessageDialog::new()
                .set_title("Voice App dialog QA")
                .set_description("Não foi possível abrir o aplicativo.")
                .set_level(rfd::MessageLevel::Error)
                .show();
            println!("{result}");
        }
        _ => panic!("expected open, save, or message"),
    }
}
