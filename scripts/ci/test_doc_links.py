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


if __name__ == "__main__":
    unittest.main()
