use std::env;
use std::{error::Error, process::ExitStatus};
use std::process::Command;

pub mod log;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    //logln!("Checking for update...");
    //let exist = Path::new("./redseat-rust").exists();
    //if !exist {
    //    download().await?;
    //}
    let mut restart = true;
    let mut retries = 0;
    while restart {
        let status = run();
        if let Ok(status) = status {
            logln!("run status: {:?}", status);
            if let Some(code) = status.code() {
                logln!("Exit code: {:?}", code);
                if code == 101{
                    if retries < 4 {
                        retries = retries + 1;
                        restart = true;
                        logln!("Panic termination will try to rerun (retry {:?}/4)", retries);
                    } else {
                        restart = false
                    }
                } else if code == 201 {
                    logln!("Restarting at the request of the server");
                    restart = true
                }
            } else {
                restart = false;
            }
        } else {
            logln!("Error running: {:?}", status);
            restart = false;
        }
    }

    Ok(())
}

async fn download() -> Result<(), Box<dyn Error>> {
    /*logln!("Downloading latest version...");
    let dld_path = "https://github.com/neckaros/redseat_rust/releases/download/main/redseat-rust";
    let response = reqwest::get(dld_path).await?;
    let mut file = File::create("./redseat-rust")?;
    let mut perms = fs::metadata("./redseat-rust")?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions("./redseat-rust", perms)?;
    let mut content =  Cursor::new(response.bytes().await?);
    copy(&mut content, &mut file)?;
*/
    Ok(())
}

fn run() -> Result<ExitStatus, Box<dyn Error>> {
    let args: Vec<String> = env::args().skip(1).collect();

    let mut command = Command::new("./redseat-rust");

    command.args(&args);

    println!("Starting command: ./redseat-rust {}", args.join(" "));

        //.arg("/dev/nonexistent")
    let status = command   .status()
        .expect("Redseat could not be executed");

    println!("ls: {status}");

    Ok(status)


}
