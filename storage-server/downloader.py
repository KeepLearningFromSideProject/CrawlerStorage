import abc
import requests
from pathlib import Path
from dataclasses import dataclass


@dataclass
class Task:
    path: str
    url: str


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
