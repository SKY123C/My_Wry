import sys

sys.path.append(r"C:\data\my_wry\dist")
import time

from application import app
from test import test as _test


app.start()

while app.is_running():
    time.sleep(0.05)

app.wait()
app.clear_handlers()
