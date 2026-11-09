import importlib.util
import pathlib
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("pipeline", pathlib.Path(__file__).with_name("m1-pipeline-summary.py"))
pipeline = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pipeline)


class PipelineTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.path = pathlib.Path(self.temp.name) / "requests.tsv"
        self.requests = ["0\t0\t1\t2\t3\t4\t5\t6\t12\t13\t4\t1\t1",
                         "1\t0\t21\t22\t23\t24\t25\t26\t32\t33\t24\t2\t2"]
        self.groups = ["1\t1\t7\t8\t9\t10\t11\t14", "2\t1\t27\t28\t29\t30\t31\t34"]

    def run_summary(self):
        self.path.write_text("caller\trevision\tstart\tadmission\tenqueue\tdequeue\tclassify\tready\tsend\twake\tadmitted\tgroup\toffset_plus_one\n" + "\n".join(self.requests) + "\n")
        self.path.with_suffix(".groups.tsv").write_text("group\tcount\tformed\tencoded\twritten\tflushed\tpublished\tdelivered\n" + "\n".join(self.groups) + "\n")
        return pipeline.summarize(self.path)

    def test_disjoint_sum(self):
        result = self.run_summary()
        self.assertEqual(result["p99_ns"], 12)
        self.assertEqual(sum(result["tail_stage_ns"].values()), result["tail_total_ns"])

    def test_refuses_missing_clock(self):
        self.requests[0] = self.requests[0].replace("\t6\t", "\t0\t")
        with self.assertRaises(AssertionError): self.run_summary()

    def test_refuses_reversed_clock(self):
        self.requests[0] = self.requests[0].replace("\t6\t", "\t16\t")
        with self.assertRaises(AssertionError): self.run_summary()

    def test_refuses_duplicate_request(self):
        self.requests[1] = self.requests[0]
        with self.assertRaises(AssertionError): self.run_summary()

    def test_refuses_wrong_group_total(self):
        self.groups[0] = self.groups[0].replace("1\t1\t", "1\t2\t")
        with self.assertRaises(AssertionError): self.run_summary()

    def test_refuses_swapped_offset_identity(self):
        self.requests[0] = self.requests[0][:-1] + "2"
        self.requests[1] = self.requests[1][:-1] + "1"
        with self.assertRaises(AssertionError): self.run_summary()


if __name__ == "__main__":
    unittest.main()
