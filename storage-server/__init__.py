import os
from pathlib import Path

from flask import Flask, request
from flask.views import View

from .data import Task
from .downloader import BackgroundDownloader
from .tasks import app as celery_app

flask_app = Flask(__name__)
downloader = BackgroundDownloader()
mount_point = Path("mnt").resolve()


@flask_app.route("/")
def hello_world():
    return "Hello, World!"


@flask_app.route("/add", methods=["POST"])
def add():
    comics = request.json
    tasks = [
        Task(
            str(
                mount_point
                / "comics"
                / comic_name
                / eposide_name
                / f"{str(num).zfill(3)}{os.path.splitext(url)[1]}"
            ),
            url,
        )
        for (comic_name, eposides) in comics.items()
        for (eposide_name, pages) in eposides.items()
        for (num, url) in enumerate(pages)
    ]
    for task in tasks:
        print(task)
        downloader.download(task)
    return {"ok": True}


@flask_app.route("/list/<comic>/<eposide>")
def list_eposide_files(comic: str, eposide: str):
    path = mount_point / "comics" / comic / eposide
    if not path.exists():
        return {"ok": False, "status": 404}
    return {"ok": True, "data": [child.name for child in path.iterdir()]}


@flask_app.route("/list/<comic>")
def list_comic_eposide(comic: str):
    path = mount_point / "comics" / comic
    if not path.exists():
        return {"ok": False, "status": 404}
    return {"ok": True, "data": [child.name for child in path.iterdir()]}


@flask_app.route("/list")
def list_comic():
    path = mount_point / "comics"
    return {"ok": True, "data": [child.name for child in path.iterdir()]}
