import unittest

import doc_links


class DocLinkTests(unittest.TestCase):
    def test_relative_links_resolve_from_the_linking_file(self):
        text = (
            "[kept](../CONTRIBUTING.md#testing) [web](https://example.com/a.md) [anchor](#here)\n"
            "[gone](performance/missing.json) [root](/docs/architecture.md \"title\")\n"
            "```\n[code](not/checked.md)\n```\n"
            "[outside](../../elsewhere.md)\n"
        )
        present = {"CONTRIBUTING.md", "docs/architecture.md"}
        self.assertEqual(doc_links.broken("docs/guide.md", text, present.__contains__), [
            "docs/guide.md:2: link target 'performance/missing.json' does not exist",
            "docs/guide.md:6: link target '../../elsewhere.md' does not exist",
        ])

    def test_escaped_spaces_name_the_file_on_disk(self):
        self.assertEqual(doc_links.broken("README.md", "[a](docs/my%20file.md)", {"docs/my file.md"}.__contains__), [])

    def test_code_fences_close_only_with_their_own_kind(self):
        text = (
            "````md\n```\n[in a fence](missing.md)\n```\n````\n"
            "~~~\n```\n[still in](missing.md)\n~~~\n"
            "use `x[i](missing.md)` and ``a `[b](missing.md)` c``\n"
            "[after](gone.md)\n"
        )
        self.assertEqual(doc_links.broken("a.md", text, set().__contains__), [
            "a.md:11: link target 'gone.md' does not exist",
        ])

    def test_query_strings_and_hosts_are_not_part_of_the_path(self):
        text = "[a](README.md?plain=1#L4) [b](//example.com/x.md) [c](docs/x.md?raw)\n"
        self.assertEqual(doc_links.broken("README.md", text, {"README.md", "docs/x.md"}.__contains__), [])

    def test_a_linked_image_checks_both_targets(self):
        text = "[![badge](docs/badge.svg)](LICENSE)\n"
        self.assertEqual(doc_links.broken("README.md", text, set().__contains__), [
            "README.md:1: link target 'docs/badge.svg' does not exist",
            "README.md:1: link target 'LICENSE' does not exist",
        ])


if __name__ == "__main__":
    unittest.main()
