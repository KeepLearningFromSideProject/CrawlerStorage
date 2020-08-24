import requests


FIXTURE_DATA = {
    "test-comic": {"ep1": ["https://robohash.org/1.jpg", "https://robohash.org/2.jpg"]}
}


def test_start_correct():
    data = requests.get("http://localhost:5050/list").json()
    assert data["ok"]


def test_add():
    data = requests.post("http://localhost:5050/add", json=FIXTURE_DATA).json()
    assert data["ok"]

    data = requests.get("http://localhost:5050/list").json()
    assert data["ok"]
    assert data["data"] == ["test-comic"]

    data = requests.get("http://localhost:5050/list/test-comic").json()
    assert data["ok"]
    assert data["data"] == ["ep1"]

    data = requests.get("http://localhost:5050/list/test-comic/ep1").json()
    assert data["ok"]
    assert data["data"] == ["000.jpg", "001.jpg"]
