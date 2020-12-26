from pathlib import Path
from typing import Dict

import requests
from celery import Celery

from .data import Task

BORKER_URL = "redis://localhost"

app = Celery("storage_server", broker=BORKER_URL)

app.conf.borker_url = BORKER_URL


@app.task
def download(task: Dict):
    task = Task(**task)
    print(task)
    res = requests.get(task.url)
    path = Path(task.path)
    path.parent.mkdir(parents=True, exist_ok=True)
    with open(path, "wb") as f:
        f.write(res.content)
