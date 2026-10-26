"""Versioned Blender API fixtures; real retained-EXR acceptance is separate."""
import ast
from pathlib import Path
from types import SimpleNamespace
import unittest

source = Path(__file__).resolve().parents[1] / "backend/blender_denoise.py"
definitions = [node for node in ast.parse(source.read_text()).body
               if isinstance(node, ast.FunctionDef) and node.name in {"compositor_tree", "configure_denoise"}]

class BlenderApiCompatibility(unittest.TestCase):
    def setUp(self):
        self.sockets = []
        self.tree = SimpleNamespace(interface=SimpleNamespace(new_socket=lambda **kw: self.sockets.append(kw)))
        self.context = {"bpy": SimpleNamespace(data=SimpleNamespace(node_groups=SimpleNamespace(new=lambda *args: self.tree)))}
        exec(compile(ast.Module(body=definitions, type_ignores=[]), str(source), "exec"), self.context)

    def test_legacy_scene_tree(self):
        scene = SimpleNamespace(node_tree=self.tree, use_nodes=False)
        tree, output = self.context["compositor_tree"](scene)
        self.assertIs(tree, self.tree)
        self.assertTrue(scene.use_nodes)
        self.assertEqual(output, "CompositorNodeComposite")
        self.assertEqual(self.sockets, [])

    def test_blender5_group_output_is_color(self):
        scene = SimpleNamespace(compositing_node_group=None)
        tree, output = self.context["compositor_tree"](scene)
        self.assertIs(scene.compositing_node_group, tree)
        self.assertEqual(output, "NodeGroupOutput")
        self.assertEqual(self.sockets, [{"name":"Image", "in_out":"OUTPUT", "socket_type":"NodeSocketColor"}])

    def test_legacy_denoise_settings(self):
        node = SimpleNamespace(inputs={})
        self.context["configure_denoise"](node)
        self.assertEqual(node.prefilter, "ACCURATE")
        self.assertTrue(node.use_hdr)

    def test_blender5_denoise_socket_settings(self):
        node = SimpleNamespace(inputs={key:SimpleNamespace(default_value=None) for key in ("HDR", "Prefilter", "Quality")})
        self.context["configure_denoise"](node)
        self.assertEqual({key:socket.default_value for key,socket in node.inputs.items()}, {"HDR":True, "Prefilter":"Accurate", "Quality":"High"})

if __name__ == "__main__":
    unittest.main()
