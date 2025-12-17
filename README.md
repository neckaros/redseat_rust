
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

Display advanced properties:
* If using a domain set the domain here
* Otherwise verify public ip is the ip of the server and port you exposed from your docker image

exemple docker file with traefik domain (replace subdomain.domain.com with your domain name):

```yaml
services:
  redseat:
    image: neckaros/redseat-rust:latest
    restart: always
    environment:
      - REDSEAT_DOMAIN=subdomain.domain.com
      - REDSEAT_NOCERT=true
    volumes:
      - redseat_config:/root/.config/redseat
    ports:
      - 8080
    networks:
      - dokploy-network
    labels:
      - "traefik.enable=true"
      - "traefik.http.routers.redseat.entrypoints=websecure"
      - "traefik.http.routers.redseat.tls.certresolver=letsencrypt"
      - "traefik.http.routers.redseat.rule=Host(`subdomain.domain.com`)"
      - "traefik.http.services.redseat.loadbalancer.server.port=8080"
networks:
  dokploy-network:
    external: true
volumes:
  redseat_config:
```

## Configs

### Paths for docker volume:
/root/.config/redseat ==> Configuration files, DBs and cache files

### Ports for docker volume:
8080 ==> Main and only port



# Setup dev env

## Windows
you need a recent version of visual studio installed for C++ builds
one-time
```bash
cargo install cargo-vcpkg
```
fetch vcpkg and build declared ports
```bash
cargo vcpkg build
```


# Setup Dev Environment old
Windows: vcpkg install libheif:x64-windows-static-md
You must have YT-DLP and FFMPEG installed (setup in your PATH)
Env variagles:
SYSTEM_DEPS_DAV1D_LINK=static
SYSTEM_DEPS_DAV1D_BUILD_INTERNAL=auto

## run with watch
cargo watch -c -w src -x "run --bin redseat-rust"
