
# Parameters
## App arguments
    serverid: force server id

    k, docker: Use docker specific settings
    m, imagesUseIm: Use image magick if installed for images conversion
    u, domain: set domain name (ex redseat.myserver.com)
    c, noCert: Don't use certificate creation (if your domain already has ssl via proxy)
    d, dir: Server configs local folder

## Env Variables
REDSEAT_SERVERID: force server id
REDSEAT_HOME: Override default address of Redseat Global Server (default is www.redseat.cloud)
REDSEAT_PORT
REDSEAT_EXP_PORT
REDSEAT_DIR: Server configs local folder
REDSEAT_DOMAIN: set domain name (ex redseat.myserver.com)
REDSEAT_NOCERT: **Boolean** | Don't use certificate creation (if your domain already has ssl via proxy)

# Docker install
Image: 
`docker pull neckaros/redseat-rust`

if you cannot see docker log simply open a webpage to:

https://www.redseat.cloud/install

Display advanced properties. Verify public ip and port you exposed from your docker image

## Configs

### Paths for docker volume:
/root/.config/redseat ==> Configuration files, DBs and cache files

### Ports for docker volume:
8080 ==> Main and only port



# Setup Dev Environment
Windows: vcpkg install libheif:x64-windows-static-md
You must have YT-DLP and FFMPEG installed (setup in your PATH)
Env variagles:
SYSTEM_DEPS_DAV1D_LINK=static
SYSTEM_DEPS_DAV1D_BUILD_INTERNAL=auto

## run with watch
cargo watch -c -w src -x "run --bin redseat-rust"
