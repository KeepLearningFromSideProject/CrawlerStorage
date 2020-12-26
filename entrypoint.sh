./comic-fs mnt &
celery -A storage-server worker &
/opt/pysetup/.venv/bin/flask run --host=0.0.0.0
