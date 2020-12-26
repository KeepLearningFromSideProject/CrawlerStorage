import abc
from dataclasses import asdict
from pathlib import Path

import requests

from . import tasks
from .data import Task


class Downloader(metaclass=abc.ABCMeta):
    @abc.abstractmethod
    def download(self, task: Task):
        "perform a download task"
        return NotImplemented


class RequestDownloader(Downloader):
    def download(self, task: Task):
        res = requests.get(task.url)
        path = Path(task.path)
        path.parent.mkdir(parents=True, exist_ok=True)
        with open(path, "wb") as f:
            f.write(res.content)


class BackgroundDownloader(Downloader):
    def download(self, task: Task):
        tasks.download.delay(asdict(task))
