"""Core supports both main(**params) and stdin-JSON subprocess entrypoints."""
try:
    from ._bridge import cli, invoke_checked
except ImportError:
    from _bridge import cli, invoke_checked
ALLOWED = {'cancel'}

def main(**params):
    return invoke_checked(ALLOWED, params)

if __name__ == '__main__':
    cli(ALLOWED)
