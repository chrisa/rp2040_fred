from interpreter import *
from emccanon import MESSAGE, SET_MOTION_OUTPUT_BIT, CLEAR_MOTION_OUTPUT_BIT,SET_AUX_OUTPUT_BIT,CLEAR_AUX_OUTPUT_BIT

def g332(self, **words):
    print(words)
    yield INTERP_OK

def g762(self, **words):
    print(words)
    yield INTERP_OK
