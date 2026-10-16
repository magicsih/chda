#!/usr/bin/env python3
"""Synthetic policy tests; these do not constitute native release evidence."""
import copy
import contextlib
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
from datetime import datetime, timedelta, timezone

SCRIPT = Path(__file__).with_name('macos-release.py')
sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location('release', SCRIPT)
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)

NOW = datetime(2026, 10, 9, 9, 0, tzinfo=timezone.utc)
SHA, TREE, CONFIG = '1' * 40, '2' * 40, '3' * 64


def timestamp(ago=0):
    return (NOW - timedelta(minutes=ago)).strftime('%Y-%m-%dT%H:%M:%SZ')


def evidence(stage='before-build'):
    run = {
        'kind': 'native-development-runtime', 'result': 'passed',
        'started_at': timestamp(10), 'completed_at': timestamp(2),
        'reviewer_sha256': '5' * 64, 'plan_sha256': '6' * 64,
        'evidence_sha256': 'e' * 64,
        'cases': sorted(release.CASES), 'features': sorted(release.FEATURES),
        'environment': {'os': 'macos', 'os_version': '26.0', 'architecture': 'arm64',
                        'claude_version': '2.1.294', 'codex_version': '0.161.0',
                        'isolated': True},
    }
    data = {'schema': 1, 'platform': 'macos', 'stage': stage,
            'candidate': {'tag': 'v0.1.21', 'sha': SHA, 'tree': TREE,
                          'config_sha256': CONFIG},
            'implementer_sha256': '4' * 64,
            'rounds': [], 'run': run}
    for number in range(1, 4):
        round_run = copy.deepcopy(run)
        round_run.update(number=number, sha=SHA, tree=TREE, config_sha256=CONFIG,
                         plan_sha256=str(number + 5) * 64,
                         evidence_sha256=str(number + 6) * 64,
                         started_at=timestamp(60 - number * 10),
                         completed_at=timestamp(55 - number * 10))
        data['rounds'].append(round_run)
    if stage == 'before-deploy':
        data['run'].update(started_at=timestamp(1), completed_at=timestamp(),
                           evidence_sha256='c' * 64)
        data['signed_artifact'] = {
            'sha256': 'a' * 64, 'evidence_sha256': 'b' * 64,
            'checks': sorted(release.ARTIFACT_CHECKS), 'result': 'passed'}
    return data


class EvidenceTests(unittest.TestCase):
    def verify(self, data, **kwargs):
        now = kwargs.pop('now', NOW)
        return release.validate_evidence(data, 'v0.1.21', SHA, TREE, CONFIG,
                                         data['stage'], now=now, **kwargs)

    def test_valid_native_evidence_and_distinct_after_build_run(self):
        before = evidence()
        self.verify(before)
        after = evidence('before-deploy')
        after['run']['started_at'] = timestamp(1)
        after['run']['completed_at'] = timestamp()
        after['run']['evidence_sha256'] = 'c' * 64
        self.verify(after, built_at=timestamp(2), artifact_sha256='a' * 64)
        after['run']['started_at'] = timestamp(3)
        with self.assertRaises(ValueError):
            self.verify(after, built_at=timestamp(2), artifact_sha256='a' * 64)

    def test_stale_future_wrong_candidate_or_config_fail_closed(self):
        with self.assertRaises(ValueError):
            self.verify(evidence(), now=NOW + timedelta(minutes=61))
        for mutate in (
            lambda d: d['run'].update(completed_at=timestamp(-1)),
            lambda d: d['candidate'].update(sha='9' * 40),
            lambda d: d['candidate'].update(config_sha256='9' * 64),
            lambda d: d['rounds'][0].update(tree='9' * 40),
        ):
            with self.subTest(mutate=mutate):
                data = evidence()
                mutate(data)
                with self.assertRaises(ValueError):
                    self.verify(data)

    def test_independent_review_and_three_real_runtime_rounds_required(self):
        for mutate in (
            lambda d: d['rounds'].pop(),
            lambda d: d['rounds'][1].update(number=1),
            lambda d: d['rounds'][0].update(reviewer_sha256='4' * 64),
            lambda d: d['run'].update(kind='headless-scenario'),
            lambda d: d['rounds'][0].update(result='blocked'),
            lambda d: d['rounds'][1].update(evidence_sha256=d['rounds'][0]['evidence_sha256']),
            lambda d: d['run']['environment'].update(isolated=False),
        ):
            data = evidence()
            mutate(data)
            with self.assertRaises(ValueError):
                self.verify(data)

    def test_every_new_case_and_existing_feature_and_private_field_rejected(self):
        for mutate in (
            lambda d: d['run']['cases'].remove('PREP-163'),
            lambda d: d['run']['cases'].remove('PICKER-181'),
            lambda d: d['rounds'][1]['cases'].remove('SESSIONS-182'),
            lambda d: d['rounds'][2]['features'].remove('Terminal'),
            lambda d: d['run'].update(prompt='private prompt'),
            lambda d: d['run']['environment'].update(path='/Users/private'),
            lambda d: d['run']['cases'].append('PREP-163'),
        ):
            data = evidence()
            mutate(data)
            with self.assertRaises(ValueError):
                self.verify(data)

    def test_deploy_requires_exact_signed_archive_and_post_build_native_checks(self):
        for mutate in (
            lambda d: d.pop('signed_artifact'),
            lambda d: d['signed_artifact'].update(sha256='d' * 64),
            lambda d: d['signed_artifact']['checks'].remove('shell-pid-preservation'),
        ):
            data = evidence('before-deploy')
            mutate(data)
            with self.assertRaises(ValueError):
                self.verify(data, artifact_sha256='a' * 64, built_at=timestamp(2))

    def test_duplicate_json_keys_and_oversized_input_rejected(self):
        with self.assertRaises(ValueError):
            release.read_json_text('{"schema":1,"schema":2}')
        with self.assertRaises(ValueError):
            release.read_json_text(' ' * 65537)


class V023CoverageTests(unittest.TestCase):
    def test_v023_rejects_historical_coverage_in_every_round_and_e2e(self):
        data = evidence()
        data['candidate']['tag'] = 'v0.1.23'
        with self.assertRaises(ValueError):
            release.validate_evidence(data, 'v0.1.23', SHA, TREE, CONFIG,
                                      'before-build', now=NOW)

    def test_v023_requires_every_issue_in_each_round_and_both_final_runs(self):
        for stage in ('before-build', 'before-deploy'):
            data = evidence(stage)
            data['candidate']['tag'] = 'v0.1.23'
            for run in [*data['rounds'], data['run']]:
                run['cases'] = sorted(release.V023_CASES)
            kwargs = dict(built_at=timestamp(2), artifact_sha256='a' * 64) if stage == 'before-deploy' else {}
            release.validate_evidence(data, 'v0.1.23', SHA, TREE, CONFIG, stage, now=NOW, **kwargs)
            for n in range(4):
                for case in release.V023_CASES - release.CASES:
                    incomplete = copy.deepcopy(data)
                    target = incomplete['rounds'][n] if n < 3 else incomplete['run']
                    target['cases'].remove(case)
                    with self.subTest(stage=stage, run=n, case=case), self.assertRaises(ValueError):
                        release.validate_evidence(incomplete, 'v0.1.23', SHA, TREE, CONFIG, stage, now=NOW, **kwargs)

    def test_historical_contract_remains_exact_and_future_versions_keep_regressions(self):
        for tag in ('v0.1.21', 'v0.1.22'):
            data=evidence()
            data['candidate']['tag']=tag
            release.validate_evidence(data, tag, SHA, TREE, CONFIG, 'before-build', now=NOW)
            data['run']['cases'].append('ISSUE-229')
            with self.assertRaises(ValueError):
                release.validate_evidence(data, tag, SHA, TREE, CONFIG, 'before-build', now=NOW)
        self.assertEqual(release.cases_for('v0.1.23'), release.V023_CASES)
        self.assertEqual(release.cases_for('v0.1.24'), release.V023_CASES)
        self.assertEqual(release.cases_for('v0.2.0'), release.V023_CASES)


class V1025CoverageTests(unittest.TestCase):
    NEW_CASES = frozenset(f'ISSUE-{n}' for n in (
        241, 242, 243, 244, 246, 247, 248, 249, 250, 251, 257))

    def test_v1025_rejects_coverage_that_omits_recent_acceptance_conditions(self):
        data = evidence()
        data['candidate']['tag'] = 'v1.0.25'
        for run in [*data['rounds'], data['run']]:
            run['cases'] = sorted(release.V023_CASES)
        with self.assertRaises(ValueError):
            release.validate_evidence(data, 'v1.0.25', SHA, TREE, CONFIG,
                                      'before-build', now=NOW)

    def test_every_recent_case_is_required_in_each_round_and_both_final_runs(self):
        expected = release.V023_CASES | self.NEW_CASES
        for stage in ('before-build', 'before-deploy'):
            data = evidence(stage)
            data['candidate']['tag'] = 'v1.0.25'
            for run in [*data['rounds'], data['run']]:
                run['cases'] = sorted(expected)
            kwargs = dict(built_at=timestamp(2), artifact_sha256='a' * 64) if stage == 'before-deploy' else {}
            release.validate_evidence(data, 'v1.0.25', SHA, TREE, CONFIG, stage, now=NOW, **kwargs)
            for n in range(4):
                for case in self.NEW_CASES:
                    incomplete = copy.deepcopy(data)
                    target = incomplete['rounds'][n] if n < 3 else incomplete['run']
                    target['cases'].remove(case)
                    with self.subTest(stage=stage, run=n, case=case), self.assertRaises(ValueError):
                        release.validate_evidence(incomplete, 'v1.0.25', SHA, TREE, CONFIG, stage, now=NOW, **kwargs)

    def test_prior_contracts_remain_exact_and_later_versions_retain_recent_cases(self):
        for tag in ('v0.1.23', 'v0.1.24', 'v0.2.0'):
            self.assertEqual(release.cases_for(tag), release.V023_CASES)
        for tag in ('v1.0.25', 'v1.0.26', 'v1.1.0', 'v2.0.0'):
            self.assertEqual(release.cases_for(tag), release.V023_CASES | self.NEW_CASES)


def exception_record(sha=SHA, tree=TREE):
    return json.dumps({
        'schema': 1, 'tag': 'v0.1.21', 'sha': sha, 'tree': tree,
        'config_sha256': CONFIG, 'owner': 'magicsih',
        'owner_instruction': '이제 좀 공개해라 좀',
        'accepted_interpretation': 'publish-with-incomplete-native-validation',
        'accepted_at': timestamp(20), 'blocked_evidence_sha256': 'b' * 64,
        'independent_assessment_sha256': 'c' * 64,
        'incomplete_checks': sorted(release.ARTIFACT_CHECKS | {
            'three-native-review-rounds', 'before-build-native-e2e',
            'before-deploy-native-e2e'}),
    }, sort_keys=True)


def exception_evidence(record, stage='before-build', sha=SHA, tree=TREE):
    approval = json.loads(record)
    data = {
        'schema': 2, 'platform': 'macos', 'stage': stage,
        'candidate': dict(tag='v0.1.21', sha=sha, tree=tree, config_sha256=CONFIG),
        'native_result': 'blocked', 'incomplete_checks': approval['incomplete_checks'],
        'exception_sha256': hashlib.sha256(record.encode()).hexdigest(),
        'checked_at': timestamp(2),
    }
    if stage == 'before-deploy':
        data.update(checked_at=timestamp(), artifact_sha256='a' * 64)
    return data


class ExceptionEvidenceTests(unittest.TestCase):
    def verify(self, data, record=None, **kwargs):
        return release.validate_evidence(
            data, 'v0.1.21', SHA, TREE, CONFIG, data['stage'], now=NOW,
            exception_record=record, **kwargs)

    def test_exact_recorded_exception_preserves_blocked_result(self):
        record = exception_record()
        data = exception_evidence(record)
        self.assertEqual(self.verify(data, record), data)
        self.assertEqual(data['native_result'], 'blocked')
        self.assertNotIn('rounds', data)
        after = exception_evidence(record, 'before-deploy')
        self.verify(after, record, built_at=timestamp(1), artifact_sha256='a' * 64)

    def test_no_main_record_wrong_record_or_candidate_cannot_authorize(self):
        record = exception_record()
        data = exception_evidence(record)
        for other in (None, exception_record(sha='9' * 40), record + '\n'):
            with self.subTest(record=other), self.assertRaises(ValueError):
                self.verify(data, other)
        other = copy.deepcopy(data)
        other['candidate']['tree'] = '9' * 40
        with self.assertRaises(ValueError):
            self.verify(other, record)
        with self.assertRaises(ValueError):
            release.validate_evidence(data, 'v0.1.22', SHA, TREE, CONFIG,
                                      'before-build', now=NOW, exception_record=record)

    def test_cannot_claim_passes_hide_incomplete_checks_or_publish_private_fields(self):
        record = exception_record()
        for mutate in (
            lambda d: d.update(native_result='passed'),
            lambda d: d['incomplete_checks'].remove('gui-ipc'),
            lambda d: d.update(rounds=[]),
            lambda d: d.update(path='/Users/private'),
            lambda d: d.update(checked_at=timestamp(-1)),
            lambda d: d.update(checked_at=timestamp(61)),
        ):
            data = exception_evidence(record)
            mutate(data)
            with self.subTest(mutate=mutate), self.assertRaises(ValueError):
                self.verify(data, record)

    def test_exception_still_requires_exact_archive_and_post_build_readback(self):
        record = exception_record()
        data = exception_evidence(record, 'before-deploy')
        with self.assertRaises(ValueError):
            self.verify(data, record, built_at=timestamp(1), artifact_sha256='d' * 64)
        with self.assertRaises(ValueError):
            self.verify(data, record, built_at=timestamp(-1), artifact_sha256='a' * 64)



class V023PublicationApprovalTests(unittest.TestCase):
    TAG = 'v0.1.23'
    INSTRUCTION = 'PR 병합하고 공개해 빨리. 아무도 쓰는 사람도 없는데 뭘 자꾸 뜸을드려 빨리 빨리 좀 해 그냥'
    # The next version must not reuse this approval.
    NEXT = 'v0.1.24'

    def approval(self):
        value = json.loads(exception_record())
        value.update(tag=self.TAG, owner_instruction=self.INSTRUCTION)
        return json.dumps(value, sort_keys=True)

    def evidence(self, record, stage='before-build'):
        value = exception_evidence(record, stage)
        value['candidate']['tag'] = self.TAG
        return value

    def verify(self, data, record, tag=None, **kwargs):
        return release.validate_evidence(data, tag or self.TAG, SHA, TREE, CONFIG,
            data['stage'], now=NOW, exception_record=record, **kwargs)

    def test_new_owner_approval_preserves_incomplete_build_and_deploy(self):
        record = self.approval()
        before = self.evidence(record)
        self.assertEqual(self.verify(before, record)['native_result'], 'blocked')
        after = self.evidence(record, 'before-deploy')
        self.verify(after, record, built_at=timestamp(1), artifact_sha256='a' * 64)

    def test_old_approval_future_tag_fake_pass_and_changed_archive_are_rejected(self):
        record = self.approval()
        for other in (None, exception_record(), reduced_exception_record()):
            with self.subTest(record=other), self.assertRaises(ValueError):
                self.verify(self.evidence(record), other)
        for mutate in (
            lambda d: d.update(native_result='passed'),
            lambda d: d['incomplete_checks'].remove('gui-ipc'),
            lambda d: d['candidate'].update(sha='9' * 40),
        ):
            data = self.evidence(record)
            mutate(data)
            with self.subTest(mutate=mutate), self.assertRaises(ValueError):
                self.verify(data, record)
        future_record = json.loads(record)
        future_record['tag'] = self.NEXT
        future_record = json.dumps(future_record, sort_keys=True)
        future = self.evidence(future_record)
        future['candidate']['tag'] = self.NEXT
        with self.assertRaises(ValueError):
            self.verify(future, future_record, tag=self.NEXT)
        with self.assertRaises(ValueError):
            self.verify(self.evidence(record, 'before-deploy'), record,
                        built_at=timestamp(1), artifact_sha256='d' * 64)


class V024PublicationApprovalTests(V023PublicationApprovalTests):
    TAG = 'v0.1.24'
    INSTRUCTION = '공개 하자'
    NEXT = 'v0.1.25'


class V1025PublicationApprovalTests(V023PublicationApprovalTests):
    TAG = 'v1.0.25'
    INSTRUCTION = 'rhdrogo.'
    NEXT = 'v1.0.26'

    def test_exact_candidate_bytes_and_fresh_readback_remain_required(self):
        record = self.approval()
        for stage in ('before-build', 'before-deploy'):
            kwargs = dict(built_at=timestamp(1), artifact_sha256='a' * 64) if stage == 'before-deploy' else {}
            for mutate in (
                lambda d: d['candidate'].update(tree='9' * 40),
                lambda d: d['candidate'].update(config_sha256='9' * 64),
                lambda d: d.update(checked_at=timestamp(61)),
                lambda d: d.update(checked_at=timestamp(-1)),
            ):
                data = self.evidence(record, stage)
                mutate(data)
                with self.subTest(stage=stage, mutate=mutate), self.assertRaises(ValueError):
                    self.verify(data, record, **kwargs)
            with self.subTest(stage=stage, changed_bytes=True), self.assertRaises(ValueError):
                self.verify(self.evidence(record, stage), record + '\n', **kwargs)

    def test_previous_publication_approval_cannot_authorize_v1025(self):
        previous = V024PublicationApprovalTests().approval()
        with self.assertRaises(ValueError):
            self.verify(self.evidence(self.approval()), previous)


class ReleaseNotesTests(unittest.TestCase):
    def test_historical_exception_notes_remain_unchanged(self):
        for tag in ('v0.1.21', 'v0.1.22', 'v0.1.23'):
            self.assertIn('https://github.com/magicsih/chda/blob/main/docs/releases/',
                          release.exception_notes(tag))
        for tag in ('v0.1.24', 'v0.1.25'):
            self.assertEqual(release.exception_notes(tag), '')

    def test_v1025_discloses_its_own_incomplete_validation_only(self):
        notes = release.exception_notes('v1.0.25')
        self.assertIn('incomplete', notes)
        self.assertIn('v1.0.25-publication-approval.md', notes)
        self.assertEqual(release.exception_notes('v1.0.26'), '')

def reduced_exception_record():
    record = json.loads(exception_record())
    record.update(tag='v0.1.22', owner_instruction='예외 출시: 반복 검수 생략 (권장)',
                  incomplete_checks=sorted(release.EXCEPTION_CHECKS - release.ARTIFACT_CHECKS),
                  observed_failures=['CHILD-161'])
    return json.dumps(record, sort_keys=True, ensure_ascii=False) + '\n'


def reduced_exception_evidence(record, stage='before-build'):
    data = exception_evidence(record, stage)
    data['candidate']['tag'] = 'v0.1.22'
    data.update(observed_failures=['CHILD-161'])
    if stage == 'before-deploy':
        data['signed_artifact'] = {
            'sha256': 'a' * 64, 'evidence_sha256': 'b' * 64,
            'checks': sorted(release.ARTIFACT_CHECKS), 'result': 'passed'}
    return data


class ReducedExceptionTests(unittest.TestCase):
    def verify(self, data, record, **kwargs):
        return release.validate_evidence(
            data, 'v0.1.22', SHA, TREE, CONFIG, data['stage'], now=NOW,
            exception_record=record, **kwargs)

    def test_reduced_exception_preserves_failure_and_requires_signed_smoke(self):
        record = reduced_exception_record()
        before = reduced_exception_evidence(record)
        self.assertEqual(self.verify(before, record), before)
        after = reduced_exception_evidence(record, 'before-deploy')
        self.assertEqual(self.verify(after, record, built_at=timestamp(1),
                                     artifact_sha256='a' * 64), after)

    def test_observed_failure_cannot_be_hidden_or_relabelled_as_passed(self):
        record = reduced_exception_record()
        for mutate in (
            lambda d: d.pop('observed_failures'),
            lambda d: d.update(observed_failures=[]),
            lambda d: d.update(native_result='passed'),
        ):
            data = reduced_exception_evidence(record)
            mutate(data)
            with self.subTest(mutate=mutate), self.assertRaises(ValueError):
                self.verify(data, record)
        altered = json.loads(record)
        altered['observed_failures'] = []
        other = json.dumps(altered, sort_keys=True)
        with self.assertRaises(ValueError):
            self.verify(reduced_exception_evidence(other), other)

    def test_owner_did_not_waive_actual_signed_artifact_checks(self):
        altered = json.loads(reduced_exception_record())
        altered['incomplete_checks'].append('session-preserving-update')
        record = json.dumps(altered, sort_keys=True)
        with self.assertRaises(ValueError):
            self.verify(reduced_exception_evidence(record), record)

    def test_deploy_refuses_missing_failed_or_wrong_archive_signed_smoke(self):
        record = reduced_exception_record()
        for mutate in (
            lambda d: d.pop('signed_artifact'),
            lambda d: d['signed_artifact'].update(result='failed'),
            lambda d: d['signed_artifact'].update(sha256='d' * 64),
            lambda d: d['signed_artifact']['checks'].remove('shell-pid-preservation'),
        ):
            data = reduced_exception_evidence(record, 'before-deploy')
            mutate(data)
            with self.subTest(mutate=mutate), self.assertRaises(ValueError):
                self.verify(data, record, built_at=timestamp(1), artifact_sha256='a' * 64)

    def test_approval_remains_candidate_bound_and_is_not_a_general_skip_option(self):
        record = reduced_exception_record()
        for key, value in (('sha', '9' * 40), ('tree', '9' * 40),
                           ('config_sha256', '9' * 64),
                           ('owner_instruction', 'skip tests')):
            altered = json.loads(record)
            altered[key] = value
            other = json.dumps(altered, sort_keys=True)
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.verify(reduced_exception_evidence(other), other)
        data = reduced_exception_evidence(record)
        with self.assertRaises(ValueError):
            release.validate_evidence(data, 'v0.1.23', SHA, TREE, CONFIG,
                                      'before-build', now=NOW, exception_record=record)


def postbuild_exception_record(original):
    return json.dumps({
        'schema': 1, 'tag': 'v0.1.22', 'sha': SHA, 'tree': TREE,
        'config_sha256': CONFIG, 'artifact_sha256': 'a' * 64,
        'original_exception_sha256': hashlib.sha256(original.encode()).hexdigest(),
        'owner': 'magicsih', 'owner_instruction': '공개해',
        'accepted_interpretation': 'publish-with-failed-isolated-update',
        'accepted_at': timestamp(2), 'failed_evidence_sha256': 'f' * 64,
        'independent_assessment_sha256': 'b' * 64,
        'signed_artifact_checks': {
            'signed-launch': 'passed', 'gui-ipc': 'passed',
            'session-preserving-update': 'failed', 'shell-pid-preservation': 'unknown'},
    }, sort_keys=True, ensure_ascii=False) + '\n'


class PostbuildExceptionTests(unittest.TestCase):
    def setUp(self):
        self.original = reduced_exception_record()
        self.postbuild = postbuild_exception_record(self.original)
        self.data = reduced_exception_evidence(self.original, 'before-deploy')
        self.data['postbuild_exception_sha256'] = hashlib.sha256(self.postbuild.encode()).hexdigest()
        self.data['signed_artifact'].update(
            result='failed', checks=json.loads(self.postbuild)['signed_artifact_checks'])

    def verify(self, **kwargs):
        return release.validate_evidence(
            self.data, 'v0.1.22', SHA, TREE, CONFIG, 'before-deploy', now=NOW,
            built_at=timestamp(3), artifact_sha256='a' * 64,
            exception_record=self.original, postbuild_exception_record=self.postbuild,
            **kwargs)

    def test_exact_postbuild_approval_preserves_original_approval_and_failed_unknown(self):
        original_digest = self.data['exception_sha256']
        self.assertEqual(self.verify(), self.data)
        self.assertEqual(self.data['exception_sha256'], original_digest)
        self.assertEqual(self.data['native_result'], 'blocked')
        self.assertEqual(self.data['signed_artifact']['checks']['session-preserving-update'], 'failed')
        self.assertEqual(self.data['signed_artifact']['checks']['shell-pid-preservation'], 'unknown')

    def test_missing_modified_or_wrong_candidate_postbuild_approval_is_rejected(self):
        original = self.postbuild
        for replacement in (None, original + '\n'):
            self.postbuild = replacement
            with self.subTest(replacement=replacement), self.assertRaises(ValueError):
                self.verify()
        for name, value in (
            ('sha', '9' * 40), ('tree', '9' * 40), ('config_sha256', '9' * 64),
            ('artifact_sha256', '9' * 64), ('original_exception_sha256', '9' * 64),
            ('owner_instruction', 'skip tests'), ('independent_assessment_sha256', '9' * 64),
            ('accepted_at', timestamp(-1)), ('accepted_at', timestamp(6))):
            altered = json.loads(original)
            altered[name] = value
            self.postbuild = json.dumps(altered, sort_keys=True, ensure_ascii=False) + '\n'
            self.data['postbuild_exception_sha256'] = hashlib.sha256(self.postbuild.encode()).hexdigest()
            with self.subTest(name=name, value=value), self.assertRaises(ValueError):
                self.verify()

    def test_observed_failed_unknown_cannot_be_masked_as_passed(self):
        for name in ('session-preserving-update', 'shell-pid-preservation'):
            original = copy.deepcopy(self.data)
            self.data['signed_artifact']['checks'][name] = 'passed'
            with self.subTest(name=name), self.assertRaises(ValueError):
                self.verify()
            self.data = original
        self.data['signed_artifact']['result'] = 'passed'
        with self.assertRaises(ValueError):
            self.verify()

    def test_postbuild_approval_cannot_waive_build_gate_or_other_tags(self):
        before = reduced_exception_evidence(self.original)
        before['postbuild_exception_sha256'] = self.data['postbuild_exception_sha256']
        with self.assertRaises(ValueError):
            release.validate_evidence(
                before, 'v0.1.22', SHA, TREE, CONFIG, 'before-build', now=NOW,
                exception_record=self.original, postbuild_exception_record=self.postbuild)
        with self.assertRaises(ValueError):
            release.validate_evidence(
                self.data, 'v0.1.23', SHA, TREE, CONFIG, 'before-deploy', now=NOW,
                built_at=timestamp(3), artifact_sha256='a' * 64,
                exception_record=self.original, postbuild_exception_record=self.postbuild)


class ArtifactTests(unittest.TestCase):
    def test_manifest_binds_archive_appcast_source_and_evidence(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            archive = root / 'chda-0.1.21-macos-universal.zip'
            archive.write_bytes(b'synthetic archive')
            signature = 'A' * 86 + '=='
            (root / 'appcast.xml').write_text(
                '<rss xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">'
                '<channel><item><sparkle:version>0.1.21</sparkle:version>'
                '<enclosure url="https://github.com/magicsih/chda/releases/download/v0.1.21/'
                f'{archive.name}" length="{archive.stat().st_size}" sparkle:edSignature="{signature}"/>'
                '</item></channel></rss>')
            before = evidence()
            manifest = release.make_manifest(root, before, now=NOW)
            release.validate_assets(root, manifest, 'v0.1.21', SHA, TREE, CONFIG)
            self.assertEqual(manifest['assets'][archive.name]['sha256'],
                             hashlib.sha256(archive.read_bytes()).hexdigest())
            archive.write_bytes(b'changed archive')
            with self.assertRaises(ValueError):
                release.validate_assets(root, manifest, 'v0.1.21', SHA, TREE, CONFIG)

    def test_no_filename_path_or_extra_asset_can_escape_manifest(self):
        manifest = {'schema': 1, 'candidate': evidence()['candidate'], 'built_at': timestamp(),
                    'assets': {'../elsewhere': {'sha256': 'a' * 64, 'size': 1}}}
        with tempfile.TemporaryDirectory() as folder:
            with self.assertRaises(ValueError):
                release.validate_assets(Path(folder), manifest, 'v0.1.21', SHA, TREE, CONFIG)


class PromotionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / 'repo'
        self.repo.mkdir()
        self.assets = self.root / 'assets'
        self.assets.mkdir()
        self.old_cwd = Path.cwd()
        os.chdir(self.repo)
        self.addCleanup(os.chdir, self.old_cwd)
        self.original_command = release.command
        self.tag = 'v0.1.21'
        self.git('init', '-b', 'main')
        self.git('config', 'user.name', 'Fixture')
        self.git('config', 'user.email', 'fixture@example.invalid')
        (self.repo / 'Cargo.toml').write_text('[workspace.package]\nversion = "0.1.21"\n')
        (self.repo / 'docs').mkdir()
        release.write_json(self.repo / 'docs/product-features.json',
                           {'features': [{'area': f} for f in sorted(release.FEATURES)]})
        self.git('add', '.')
        self.git('commit', '-m', 'feat: synthetic fixture')
        self.sha = self.git('rev-parse', 'HEAD')
        self.tree = self.git('rev-parse', 'HEAD^{tree}')
        self.git('tag', 'v0.1.21')
        self.git('update-ref', 'refs/remotes/origin/main', self.sha)
        self.before = self.bind(evidence())
        archive = self.assets / release.archive_name('v0.1.21')
        archive.write_bytes(b'synthetic archive')
        (self.assets / 'appcast.xml').write_text(
            '<rss xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">'
            '<channel><item><sparkle:version>0.1.21</sparkle:version>'
            '<enclosure url="https://github.com/magicsih/chda/releases/download/v0.1.21/'
            f'{archive.name}" length="{archive.stat().st_size}" sparkle:edSignature="{"A" * 86}=="/>'
            '</item></channel></rss>')
        self.manifest = release.make_manifest(self.assets, self.before, now=NOW - timedelta(minutes=2))
        self.after = self.bind(evidence('before-deploy'))
        self.after['signed_artifact']['sha256'] = release.hash_file(archive)
        self.state = {'draft': True, 'prerelease': False, 'tag_name': 'v0.1.21'}
        self.previous_tag = 'v0.1.20'
        self.mutations = []
        self.signed_archive_checks = []
        self.signed_archive_failure = False

    def git(self, *args):
        return self.original_command('git', *args)

    def bind(self, data):
        data['candidate'].update(sha=self.sha, tree=self.tree)
        for review in data['rounds']:
            review.update(sha=self.sha, tree=self.tree)
        return data

    def command(self, *args):
        if args[0] == 'git':
            return self.original_command(*args)
        if args[:3] == ('gh', 'api', 'repos/magicsih/chda/releases/latest'):
            return json.dumps({'tag_name': self.previous_tag if self.state['draft'] else self.tag,
                               'draft': False, 'prerelease': False})
        if args[:3] == ('gh', 'release', 'upload'):
            self.mutations.append('upload')
            shutil.copy2(args[4], self.assets / 'before-deploy.json')
            return ''
        if args[:3] == ('gh', 'release', 'edit'):
            self.mutations.append('publish')
            self.state['draft'] = False
            return ''
        self.fail('Unexpected command in synthetic promotion test')

    def run_stage(self, stage, data=None, now=NOW, exception=None, postbuild_exception=None):
        env = {'GITHUB_REPOSITORY': release.REPO, 'GITHUB_EVENT_NAME': 'workflow_dispatch',
               'GITHUB_REF': 'refs/heads/main', 'RELEASE_TAG': self.tag,
               'RELEASE_SHA': self.sha, 'RELEASE_CONFIG_SHA256': CONFIG,
               'RELEASE_TOKEN': 'synthetic-test-only',
               'NATIVE_EVIDENCE': json.dumps(data if data is not None else self.after)}

        def download(root, _tag):
            for path in self.assets.iterdir():
                shutil.copy2(path, root / path.name)

        def signed_archive(_root, tag):
            self.signed_archive_checks.append(tag)
            if self.signed_archive_failure:
                raise ValueError('Synthetic signed archive verification failed')

        with contextlib.redirect_stdout(io.StringIO()), \
                patch.dict(os.environ, env), patch.object(sys, 'argv', [str(SCRIPT), stage]), \
                patch.object(release, 'command', self.command), \
                patch.object(release, 'gh_release', lambda _tag: self.state), \
                patch.object(release, 'download', download), \
                patch.object(release, 'frozen_exception', lambda: exception), \
                patch.object(release, 'frozen_postbuild_exception', lambda: postbuild_exception), \
                patch.object(release, 'native_archive', signed_archive), \
                patch.object(release, 'update_cask', lambda *_: self.mutations.append('cask')), \
                patch.object(release, 'datetime', wraps=datetime) as clock:
            clock.now.return_value = now
            release.main()

    def prepare_postbuild_exception(self):
        self.tag = 'v0.1.22'
        (self.repo / 'Cargo.toml').write_text('[workspace.package]\nversion = "0.1.22"\n')
        self.git('add', '.')
        self.git('commit', '-m', 'chore(release): synthetic candidate')
        self.sha = self.git('rev-parse', 'HEAD')
        self.tree = self.git('rev-parse', 'HEAD^{tree}')
        self.git('tag', self.tag)
        self.git('update-ref', 'refs/remotes/origin/main', self.sha)
        self.state['tag_name'] = self.tag
        self.previous_tag = 'v0.1.21'
        archive = self.assets / release.archive_name(self.tag)
        (self.assets / release.archive_name('v0.1.21')).rename(archive)
        feed = self.assets / 'appcast.xml'
        feed.write_text(feed.read_text().replace('0.1.21', '0.1.22'))
        record = json.loads(reduced_exception_record())
        record.update(sha=self.sha, tree=self.tree)
        original = json.dumps(record, sort_keys=True, indent=2, ensure_ascii=False) + '\n'
        self.before = reduced_exception_evidence(original)
        self.before['candidate'].update(sha=self.sha, tree=self.tree)
        self.manifest = release.make_manifest(self.assets, self.before, now=NOW - timedelta(minutes=2))
        self.after = reduced_exception_evidence(original, 'before-deploy')
        self.after['candidate'].update(sha=self.sha, tree=self.tree)
        supplemental = json.loads(postbuild_exception_record(original))
        supplemental.update(sha=self.sha, tree=self.tree, artifact_sha256=release.hash_file(archive),
                            accepted_at=timestamp(1))
        postbuild = json.dumps(supplemental, sort_keys=True, indent=2, ensure_ascii=False) + '\n'
        self.after['postbuild_exception_sha256'] = hashlib.sha256(postbuild.encode()).hexdigest()
        self.after['artifact_sha256'] = release.hash_file(archive)
        self.after['signed_artifact'].update(sha256=release.hash_file(archive), result='failed',
                                            checks=copy.deepcopy(release.POSTBUILD_CHECKS))
        return original, postbuild

    def test_postbuild_promotion_retains_all_draft_bytes_and_truthful_failed_unknown(self):
        original, postbuild = self.prepare_postbuild_exception()
        hashes = {p.name: release.hash_file(p) for p in self.assets.iterdir()}
        self.run_stage('publish', exception=original, postbuild_exception=postbuild)
        self.assertEqual(self.mutations, ['upload', 'publish', 'cask'])
        self.assertEqual(self.signed_archive_checks, ['v0.1.22'])
        self.assertEqual({name: release.hash_file(self.assets / name) for name in hashes}, hashes)
        after = release.read_json(self.assets / 'before-deploy.json')
        self.assertEqual(after['exception_sha256'], self.before['exception_sha256'])
        self.assertEqual(after['signed_artifact']['result'], 'failed')
        self.assertEqual(after['signed_artifact']['checks'], release.POSTBUILD_CHECKS)
        self.run_stage('cask', exception=original, postbuild_exception=postbuild)
        self.assertEqual(self.mutations, ['upload', 'publish', 'cask', 'cask'])

    def test_postbuild_approval_never_bypasses_signed_archive_failure(self):
        original, postbuild = self.prepare_postbuild_exception()
        self.signed_archive_failure = True
        with self.assertRaisesRegex(ValueError, 'signed archive verification failed'):
            self.run_stage('publish', exception=original, postbuild_exception=postbuild)
        self.assertEqual(self.mutations, [])
        self.assertTrue(self.state['draft'])

    def test_postbuild_freeze_preserves_both_committed_approval_bytes(self):
        original, postbuild = self.prepare_postbuild_exception()
        first = self.repo / release.EXCEPTION_PATHS[self.tag]
        second = self.repo / release.POSTBUILD_EXCEPTION_PATH
        first.parent.mkdir(parents=True, exist_ok=True)
        first.write_text(original, encoding='utf-8')
        second.write_text(postbuild, encoding='utf-8')
        self.git('add', '.')
        self.git('commit', '-m', 'docs(release): synthetic approvals')
        gate = self.root / 'frozen-gate.py'
        with patch.object(release, '__file__', str(gate)):
            release.freeze_exception(self.tag)
            release.freeze_postbuild_exception(self.tag)
            second.write_text(postbuild + '\n', encoding='utf-8')
            self.assertEqual(release.frozen_exception(), original)
            self.assertEqual(release.frozen_postbuild_exception(), postbuild)
            self.assertEqual(gate.with_suffix('.postbuild-exception.json').stat().st_mode & 0o777, 0o600)
            with self.assertRaises(ValueError):
                release.freeze_postbuild_exception(self.tag)

    def test_fresh_build_check_and_queue_expiration_create_no_release(self):
        self.state = None
        self.run_stage('check-build', self.before)
        self.assertEqual(self.mutations, [])
        with self.assertRaisesRegex(ValueError, 'older than 60 minutes'):
            self.run_stage('check-build', self.before, now=NOW + timedelta(minutes=61))
        self.assertEqual(self.mutations, [])

    def test_older_version_cannot_replace_latest(self):
        self.previous_tag = 'v0.1.22'
        with self.assertRaisesRegex(ValueError, 'equal or older version'):
            self.run_stage('publish')
        self.assertEqual(self.mutations, ['upload'])
        self.assertTrue(self.state['draft'])

    def test_missing_native_coverage_or_modified_archive_cannot_publish(self):
        data = copy.deepcopy(self.after)
        data['run']['cases'].remove('COPY-158')
        with self.assertRaises(ValueError):
            self.run_stage('publish', data)
        self.assertEqual(self.mutations, [])
        (self.assets / release.archive_name('v0.1.21')).write_bytes(b'tampered')
        with self.assertRaises(ValueError):
            self.run_stage('publish')
        self.assertEqual(self.mutations, [])

    def test_publish_same_draft_once_then_retry_or_cask_without_rebuilding(self):
        self.run_stage('publish')
        self.assertEqual(self.mutations, ['upload', 'publish', 'cask'])
        self.assertEqual(self.signed_archive_checks, ['v0.1.21'])
        self.mutations.clear()
        self.run_stage('publish')
        self.assertEqual(self.mutations, ['cask'])
        self.mutations.clear()
        self.run_stage('cask')
        self.assertEqual(self.mutations, ['cask'])

    def test_changed_public_evidence_is_not_replaced(self):
        self.run_stage('publish')
        self.mutations.clear()
        data = copy.deepcopy(self.after)
        data['run']['evidence_sha256'] = 'd' * 64
        with self.assertRaises(ValueError):
            self.run_stage('publish', data)
        self.assertEqual(self.mutations, [])

    def test_wrong_tag_version_tree_and_implicit_push_fail_closed(self):
        with self.assertRaises(ValueError):
            release.check_source('v0.1.21', 'f' * 40, CONFIG)
        self.git('tag', 'v0.1.22')
        with self.assertRaises(ValueError):
            release.check_source('v0.1.22', self.sha, CONFIG)
        data = copy.deepcopy(self.before)
        data['rounds'][0]['tree'] = 'f' * 40
        with self.assertRaises(ValueError):
            release.check_round_commits(data)
        with patch.dict(os.environ, {'GITHUB_EVENT_NAME': 'push'}):
            with self.assertRaises(ValueError):
                release.workflow_context()

    def test_exception_promotion_retains_blocked_result_and_same_signed_asset_checks(self):
        record = exception_record(self.sha, self.tree)
        self.before = exception_evidence(record, sha=self.sha, tree=self.tree)
        release.make_manifest(self.assets, self.before, now=NOW - timedelta(minutes=1))
        after = exception_evidence(record, 'before-deploy', sha=self.sha, tree=self.tree)
        after['artifact_sha256'] = release.hash_file(self.assets / release.archive_name('v0.1.21'))
        with self.assertRaises(ValueError):
            self.run_stage('publish', after)
        self.assertEqual(self.mutations, [])
        self.run_stage('publish', after, exception=record)
        self.assertEqual(self.mutations, ['upload', 'publish', 'cask'])
        deployed = release.read_json(self.assets / 'before-deploy.json')
        self.assertEqual(deployed['native_result'], 'blocked')
        self.assertEqual(deployed, after)
        self.mutations.clear()
        self.run_stage('cask', exception=record)
        self.assertEqual(self.mutations, ['cask'])
        self.assertEqual(self.signed_archive_checks, ['v0.1.21', 'v0.1.21'])

    def test_exception_cannot_publish_when_signed_archive_verification_fails(self):
        record = exception_record(self.sha, self.tree)
        before = exception_evidence(record, sha=self.sha, tree=self.tree)
        release.make_manifest(self.assets, before, now=NOW - timedelta(minutes=1))
        after = exception_evidence(record, 'before-deploy', sha=self.sha, tree=self.tree)
        after['artifact_sha256'] = release.hash_file(self.assets / release.archive_name('v0.1.21'))
        self.signed_archive_failure = True
        with self.assertRaisesRegex(ValueError, 'signed archive verification failed'):
            self.run_stage('publish', after, exception=record)
        self.assertEqual(self.mutations, [])
        self.assertTrue(self.state['draft'])

    def test_exception_modified_archive_cannot_publish(self):
        record = exception_record(self.sha, self.tree)
        before = exception_evidence(record, sha=self.sha, tree=self.tree)
        release.make_manifest(self.assets, before, now=NOW - timedelta(minutes=1))
        after = exception_evidence(record, 'before-deploy', sha=self.sha, tree=self.tree)
        archive = self.assets / release.archive_name('v0.1.21')
        after['artifact_sha256'] = release.hash_file(archive)
        archive.write_bytes(b'tampered')
        with self.assertRaises(ValueError):
            self.run_stage('publish', after, exception=record)
        self.assertEqual(self.mutations, [])

    def test_main_record_is_frozen_before_candidate_checkout_and_cannot_be_replaced(self):
        record = json.dumps(json.loads(exception_record(self.sha, self.tree)),
                            sort_keys=True, indent=2, ensure_ascii=False) + '\n'
        doc = self.repo / release.EXCEPTION_PATH
        doc.parent.mkdir(parents=True)
        doc.write_text(record)
        gate = self.root / 'gate.py'
        with patch.object(release, '__file__', str(gate)):
            # An uncommitted candidate/working-directory file cannot authorize publication.
            release.freeze_exception('v0.1.21')
            self.assertIsNone(release.frozen_exception())
            self.git('add', str(doc.relative_to(self.repo)))
            self.git('commit', '-m', 'chore: synthetic main approval')
            release.freeze_exception('v0.1.21')
            self.git('switch', '--detach', self.sha)
            self.assertFalse(doc.exists())
            self.assertEqual(release.frozen_exception(), record)
            self.assertEqual(gate.with_suffix('.exception.json').stat().st_mode & 0o777, 0o600)
            with self.assertRaises(ValueError):
                release.freeze_exception('v0.1.21')

    def test_main_record_original_bytes_are_not_whitespace_normalized(self):
        record = json.dumps(json.loads(exception_record(self.sha, self.tree)),
                            sort_keys=True, indent=2, ensure_ascii=False) + '\n'
        doc = self.repo / release.EXCEPTION_PATH
        doc.parent.mkdir(parents=True)
        for number, noncanonical in enumerate((' \n' + record, record + '\n')):
            doc.write_text(noncanonical)
            self.git('add', str(doc.relative_to(self.repo)))
            self.git('commit', '-m', 'chore: synthetic noncanonical approval')
            gate = self.root / ('gate-%d.py' % number)
            with self.subTest(number=number), patch.object(release, '__file__', str(gate)):
                with self.assertRaises(ValueError):
                    release.freeze_exception('v0.1.21')
                self.assertFalse(gate.with_suffix('.exception.json').exists())

    def test_reduced_approval_is_frozen_from_main_before_candidate_checkout(self):
        value = json.loads(reduced_exception_record())
        value.update(sha=self.sha, tree=self.tree)
        record = json.dumps(value, sort_keys=True, indent=2, ensure_ascii=False) + '\n'
        doc = self.repo / release.EXCEPTION_PATHS['v0.1.22']
        doc.parent.mkdir(parents=True)
        doc.write_text(record)
        gate = self.root / 'reduced-gate.py'
        with patch.object(release, '__file__', str(gate)):
            release.freeze_exception('v0.1.22')
            self.assertIsNone(release.frozen_exception())
            self.git('add', str(doc.relative_to(self.repo)))
            self.git('commit', '-m', 'chore: synthetic reduced approval')
            release.freeze_exception('v0.1.22')
            self.git('switch', '--detach', self.sha)
            self.assertFalse(doc.exists())
            self.assertEqual(release.frozen_exception(), record)


class CaskTests(unittest.TestCase):
    def test_no_duplicate_write_readback_latest_guard_and_api_error(self):
        project = SCRIPT.parent.parent
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            archive = root / 'fixture.zip'
            archive.write_bytes(b'cask fixture')
            rendered = (project / 'packaging/homebrew/chda.rb').read_text().replace(
                '__VERSION__', '0.1.21').replace('__SHA256__', release.hash_file(archive))
            state, calls = root / 'cask.rb', root / 'calls'
            state.write_text(rendered)
            mock = root / 'gh'
            mock.write_text('''#!/usr/bin/env python3
import base64, json, os, pathlib, sys
args = sys.argv[1:]
root = pathlib.Path(os.environ['CHDA_TEST_ROOT'])
if 'repos/magicsih/chda/releases/latest' in args:
    print(os.environ.get('CHDA_TEST_LATEST', 'v0.1.21'))
elif '-X' in args:
    content = next(a.removeprefix('content=') for a in args if a.startswith('content='))
    (root / 'cask.rb').write_bytes(base64.b64decode(content))
    with (root / 'calls').open('a') as f: f.write('PUT\\n')
    print('fixture-commit')
elif os.environ.get('CHDA_TEST_FAIL_GET'):
    sys.exit(1)
else:
    content = base64.b64encode((root / 'cask.rb').read_bytes()).decode()
    print(content if '--jq' in args else json.dumps({'sha': 'fixture-sha', 'content': content}))
''')
            mock.chmod(0o700)
            env = {**os.environ, 'PATH': str(root) + os.pathsep + os.environ['PATH'],
                   'CHDA_TEST_ROOT': str(root)}
            argv = ['bash', str(project / 'scripts/update-cask.sh'), '0.1.21', str(archive)]
            self.assertEqual(subprocess.run(argv, env=env, capture_output=True).returncode, 0)
            self.assertFalse(calls.exists())
            state.write_text('old cask')
            self.assertEqual(subprocess.run(argv, env=env, capture_output=True).returncode, 0)
            self.assertEqual(calls.read_text(), 'PUT\n')
            self.assertEqual(state.read_text(), rendered)
            self.assertEqual(subprocess.run(argv, env=env, capture_output=True).returncode, 0)
            self.assertEqual(calls.read_text(), 'PUT\n')
            for extra in ({'CHDA_TEST_LATEST': 'v0.1.22'}, {'CHDA_TEST_FAIL_GET': '1'}):
                self.assertNotEqual(subprocess.run(argv, env={**env, **extra}, capture_output=True).returncode, 0)
            self.assertEqual(calls.read_text(), 'PUT\n')


class SignatureTests(unittest.TestCase):
    @unittest.skipUnless(sys.platform == 'darwin', 'Apple CryptoKit requires macOS')
    def test_real_ed25519_accepts_original_rejects_modified_archive_and_wrong_key(self):
        # The key exists only in the Swift fixture process. Its private bytes are
        # never returned, logged, written to disk or taken from production.
        generator = '''import CryptoKit
import Foundation
let key = Curve25519.Signing.PrivateKey()
let wrong = Curve25519.Signing.PrivateKey()
let body = Data("synthetic archive".utf8)
print(key.publicKey.rawRepresentation.base64EncodedString())
print(try key.signature(for: body).base64EncodedString())
print(wrong.publicKey.rawRepresentation.base64EncodedString())
'''
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            fixture = root / 'fixture.swift'
            fixture.write_text(generator)
            generated = subprocess.run(['xcrun', 'swift', str(fixture)], check=True,
                                       capture_output=True, text=True).stdout.splitlines()
            public_key, signature, wrong_key = generated
            archive = root / 'fixture.zip'
            archive.write_bytes(b'synthetic archive')
            verifier = SCRIPT.with_name('verify-macos-update.swift')
            argv = ['xcrun', 'swift', str(verifier), str(archive), signature, public_key]
            self.assertEqual(subprocess.run(argv, capture_output=True).returncode, 0)
            self.assertNotEqual(subprocess.run([*argv[:-1], wrong_key], capture_output=True).returncode, 0)
            archive.write_bytes(b'modified archive')
            self.assertNotEqual(subprocess.run(argv, capture_output=True).returncode, 0)


if __name__ == '__main__':
    unittest.main()
