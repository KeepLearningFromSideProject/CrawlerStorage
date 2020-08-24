FROM rust:1.45 as build

RUN apt update && apt install libsqlite3-dev libfuse3-dev && cargo install diesel_cli --no-default-features --features sqlite

COPY . .

RUN cargo build --release

# `python-base` sets up all our shared environment variables
FROM python:3.8-slim-buster as python-base

    # python
ENV PYTHONUNBUFFERED=1 \
    # prevents python creating .pyc files
    PYTHONDONTWRITEBYTECODE=1 \
    \
    # pip
    PIP_NO_CACHE_DIR=off \
    PIP_DISABLE_PIP_VERSION_CHECK=on \
    PIP_DEFAULT_TIMEOUT=100 \
    \
    # poetry
    # https://python-poetry.org/docs/configuration/#using-environment-variables
    POETRY_VERSION=1.0.3 \
    # make poetry install to this location
    POETRY_HOME="/opt/poetry" \
    # make poetry create the virtual environment in the project's root
    # it gets named `.venv`
    POETRY_VIRTUALENVS_IN_PROJECT=true \
    # do not ask any interactive question
    POETRY_NO_INTERACTION=1 \
    \
    # paths
    # this is where our requirements + virtual environment will live
    PYSETUP_PATH="/opt/pysetup" \
    VENV_PATH="/opt/pysetup/.venv"


# prepend poetry and venv to path
ENV PATH="$POETRY_HOME/bin:$VENV_PATH/bin:$PATH"


# `builder-base` stage is used to build deps + create our virtual environment
FROM python-base as builder-base
RUN apt-get update \
    && apt-get install --no-install-recommends -y \
        # deps for installing poetry
        curl \
        # deps for building python deps
        build-essential

# install poetry - respects $POETRY_VERSION & $POETRY_HOME
RUN curl -sSL https://raw.githubusercontent.com/sdispater/poetry/master/get-poetry.py | python

# copy project requirement files here to ensure they will be cached.
WORKDIR $PYSETUP_PATH
COPY poetry.lock pyproject.toml ./

# install runtime deps - uses $POETRY_VIRTUALENVS_IN_PROJECT internally
RUN poetry install --no-dev

FROM python-base as production
EXPOSE 5000
RUN apt-get update \
    && apt-get install --no-install-recommends -y libfuse3-3 \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /storage/files
COPY migrations  /migrations/
COPY entrypoint.sh Cargo.toml ./
COPY --from=build /target/release/comic-fs /usr/local/cargo/bin/diesel .flaskenv .env ./
COPY --from=builder-base $PYSETUP_PATH $PYSETUP_PATH
COPY ./storage-server /storage-server/
CMD ["bash", "entrypoint.sh"]
