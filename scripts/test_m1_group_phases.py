import importlib.util
import pathlib
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("phases", pathlib.Path(__file__).with_name("m1-group-phases.py"))
phases = importlib.util.module_from_spec(spec)
spec.loader.exec_module(phases)

REQUESTS = "caller\trevision\tstart\tadmission\tenqueue\tdequeue\tclassify\tready\tsend\twake\tadmitted\tgroup\toffset_plus_one\n"
GROUPS = "group\tcount\tformed\tencoded\twritten\tflushed\tpublished\tdelivered\n"


class GroupPhasesTest(unittest.TestCase):
    def summarize(self, requests, groups, min_count=1):
        with tempfile.TemporaryDirectory() as temp:
            path = pathlib.Path(temp) / "requests.tsv"
            path.write_text(REQUESTS + "\n".join(requests) + "\n")
            path.with_suffix(".groups.tsv").write_text(GROUPS + "\n".join(groups) + "\n")
            return phases.summarize(path, min_count)

    def test_phases_partition_the_group_and_the_request(self):
        requests = ["0\t0\t1\t2\t3\t4\t5\t6\t12\t15\t4\t1\t1", "1\t0\t2\t3\t4\t6\t7\t8\t13\t14\t5\t1\t2",
                    "0\t1\t20\t21\t22\t24\t25\t26\t33\t36\t23\t3\t3"]
        groups = ["1\t2\t8\t9\t10\t11\t12\t14", "3\t1\t27\t29\t30\t32\t33\t34"]
        total, group, request = self.summarize(requests, groups)
        self.assertEqual(total, 2)
        self.assertEqual(group["formation"], [4, 3])
        self.assertEqual(group["delivery"], [2, 1])
        self.assertEqual(group["cycle"], [20])
        self.assertEqual(sum(group[name][0] for name in phases.GROUP_PHASES), 14 - 4)
        self.assertEqual(request["answer_to_wake"], [3, 1, 3])
        for index in range(3):
            self.assertEqual(sum(request[name][index] for name in phases.REQUEST_PHASES),
                             request["total"][index])

    def test_small_groups_do_not_count_or_bridge_a_cycle(self):
        requests = ["0\t0\t1\t2\t3\t4\t5\t6\t12\t15\t4\t1\t1", "1\t0\t2\t3\t4\t6\t7\t8\t13\t14\t5\t1\t2",
                    "0\t1\t20\t21\t22\t24\t25\t26\t33\t36\t23\t3\t3"]
        groups = ["1\t2\t8\t9\t10\t11\t12\t14", "3\t1\t27\t29\t30\t32\t33\t34"]
        _, group, _ = self.summarize(requests, groups, min_count=2)
        self.assertEqual(group["formation"], [4])
        self.assertEqual(group["cycle"], [])

    def test_refuses_reversed_group_clock(self):
        requests = ["0\t0\t1\t2\t3\t4\t5\t6\t12\t15\t4\t1\t1"]
        with self.assertRaises(AssertionError):
            self.summarize(requests, ["1\t1\t8\t7\t10\t11\t12\t14"])


if __name__ == "__main__":
    unittest.main()
