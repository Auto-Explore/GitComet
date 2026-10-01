import copy
import unittest
from public_api import public_api


class PublicApiTests(unittest.TestCase):
    def fixture(self, first=1):
        def item(offset, name, inner, visibility="public"):
            return {"id": first+offset, "crate_id": 0, "name": name, "inner": inner,
                    "visibility": visibility, "attrs": [], "docs": "words", "span": {"begin": [3,1]}}
        items = [item(0,"api",{"module":{"items":[first+1,first+3]}}),
                 item(1,"Thing",{"struct":{"kind":{"plain":{"fields":[first+2]}},"impls":[]}}),
                 item(2,"value",{"struct_field":{"primitive":"u32"}}),
                 item(3,"secret",{"function":{"has_body":True}},"default")]
        return {"root":first, "paths":{}, "index":{str(item["id"]):item for item in items}}

    def test_ids_source_locations_docs_and_private_items_are_not_contract(self):
        before = public_api(self.fixture())
        after = self.fixture(500)
        for item in after["index"].values():
            item["docs"] = "changed docs"
            item["span"] = {"begin":[50,10]}
        self.assertEqual(before,public_api(after))
        self.assertEqual(list(before),["api::Thing"])

    def test_public_field_type_changes_are_detected(self):
        before = self.fixture()
        after = copy.deepcopy(before)
        after["index"]["3"]["inner"]["struct_field"] = {"primitive":"u64"}
        self.assertNotEqual(public_api(before),public_api(after))

    def test_reexports_share_one_definition_and_keep_the_alias(self):
        doc = self.fixture()
        doc["index"]["4"].update(visibility="public", name="ThingAlias",
                                 inner={"use":{"id":2,"name":"ThingAlias","is_glob":False}})
        result = public_api(doc)
        self.assertIn("definition", result["api::Thing"])
        self.assertEqual(result["api::ThingAlias"], {"reexport":"api::Thing"})

    def test_external_reexports_are_part_of_the_contract(self):
        doc = self.fixture()
        doc["index"]["4"].update(visibility="public", name="Source",
                                 inner={"use":{"id":999,"name":"Source","is_glob":False}})
        doc["paths"]["999"] = {"path":["state","ChangeSource"]}
        self.assertEqual(public_api(doc)["api::Source"], {"reexport":"state::ChangeSource"})
