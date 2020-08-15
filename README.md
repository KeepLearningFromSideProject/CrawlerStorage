CrawlerStorage
=============

Endpoint
--------

- `/add`:
  body:
  ```json
  {
    "<comic name>": {
      "<eposide>": [
        "<url1>",
        "<url2>",
        "<url3>"
      ]
    }
  }
  ```
- `/list`:
  response:
  ```json
  {
    "ok": true,
    "data": [
      "comic1"
    ]
  }
  ```
- `/list/<comic_name>`:
  response:
  ```json
  {
    "ok": true,
    "data": [
      "ep1"
    ]
  }
  ```
- `/list/<comic_name>/<ep_name>`:
  response:
  ```json
  {
    "ok": true,
    "data": [
      "file1"
    ]
  }
  ```

Build
-----

```shell
$ ./build.sh
```

Start the service
-----------------

```shell
$ ./run.sh
```

Development
-----------

### requirements:

filesystem:

- rust >= 1.45
- libsqlite3-dev
- libfuse3-dev
- fuse3

server:

- [poetry](https://python-poetry.org)


### run on local:

build and install dependencies:

```shell
$ cargo build --release
$ poetry install
```

run:

```shell
$ poetry shell # enter virtual environment
$ cargo run --release mnt &
$ flask run
```
