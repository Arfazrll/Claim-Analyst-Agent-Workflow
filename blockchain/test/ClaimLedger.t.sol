// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ClaimLedger} from "../src/ClaimLedger.sol";

/// Minimal cheat-code interface so the tests need no external library.
interface Vm {
    function prank(address) external;
    function expectRevert(bytes calldata) external;
    function expectRevert(bytes4) external;
    function warp(uint256) external;
}

contract ClaimLedgerTest {
    Vm internal constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    ClaimLedger internal ledger;
    address internal constant WRITER = address(0xBEEF);
    address internal constant STRANGER = address(0xBAD);
    bytes32 internal constant RECORD = keccak256("claim:1");
    bytes32 internal constant HASH_A = keccak256("entry-a");
    bytes32 internal constant HASH_B = keccak256("entry-b");

    function setUp() public {
        ledger = new ClaimLedger(WRITER);
    }

    function _eq(uint256 a, uint256 b, string memory why) internal pure {
        require(a == b, why);
    }

    function test_DeployerAndInitialWriterCanWrite() public view {
        require(ledger.owner() == address(this), "owner");
        require(ledger.writers(address(this)), "deployer writer");
        require(ledger.writers(WRITER), "initial writer");
        require(!ledger.writers(STRANGER), "stranger");
    }

    function test_AnchorAppendsHistory() public {
        vm.warp(1_700_000_000);
        vm.prank(WRITER);
        uint256 i0 = ledger.anchor(RECORD, HASH_A, 1, "bafy-cid-a");
        vm.prank(WRITER);
        uint256 i1 = ledger.anchor(RECORD, HASH_B, 3, "");
        _eq(i0, 0, "first index");
        _eq(i1, 1, "second index");
        _eq(ledger.entryCount(RECORD), 2, "count");
        _eq(ledger.totalEntries(), 2, "total");
        require(ledger.isAnchored(HASH_A) && ledger.isAnchored(HASH_B), "anchored");
        ClaimLedger.Entry memory e = ledger.getEntry(RECORD, 0);
        require(e.dataHash == HASH_A && e.kind == 1 && e.submitter == WRITER, "entry fields");
        _eq(e.timestamp, 1_700_000_000, "timestamp");
        require(keccak256(bytes(e.cid)) == keccak256("bafy-cid-a"), "cid");
        require(ledger.latest(RECORD).dataHash == HASH_B, "latest");
        require(ledger.verify(RECORD, 1, HASH_B), "verify true");
        require(!ledger.verify(RECORD, 1, HASH_A), "verify wrong hash");
        require(!ledger.verify(RECORD, 9, HASH_A), "verify out of range");
    }

    function test_RevertWhen_StrangerAnchors() public {
        vm.prank(STRANGER);
        vm.expectRevert(ClaimLedger.NotWriter.selector);
        ledger.anchor(RECORD, HASH_A, 1, "");
    }

    function test_RevertWhen_HashReused() public {
        ledger.anchor(RECORD, HASH_A, 1, "");
        vm.expectRevert(abi.encodeWithSelector(ClaimLedger.AlreadyAnchored.selector, HASH_A));
        ledger.anchor(keccak256("claim:2"), HASH_A, 1, "");
    }

    function test_RevertWhen_InvalidInput() public {
        vm.expectRevert(ClaimLedger.ZeroHash.selector);
        ledger.anchor(RECORD, bytes32(0), 1, "");
        vm.expectRevert(abi.encodeWithSelector(ClaimLedger.InvalidKind.selector, uint8(0)));
        ledger.anchor(RECORD, HASH_A, 0, "");
        bytes memory longCid = new bytes(129);
        vm.expectRevert(ClaimLedger.CidTooLong.selector);
        ledger.anchor(RECORD, HASH_A, 1, string(longCid));
    }

    function test_PauseBlocksWrites() public {
        ledger.setPaused(true);
        vm.expectRevert(ClaimLedger.ContractPaused.selector);
        ledger.anchor(RECORD, HASH_A, 1, "");
        ledger.setPaused(false);
        ledger.anchor(RECORD, HASH_A, 1, "");
    }

    function test_OnlyOwnerManagesWriters() public {
        vm.prank(STRANGER);
        vm.expectRevert(ClaimLedger.NotOwner.selector);
        ledger.setWriter(STRANGER, true);
        ledger.setWriter(WRITER, false);
        vm.prank(WRITER);
        vm.expectRevert(ClaimLedger.NotWriter.selector);
        ledger.anchor(RECORD, HASH_A, 1, "");
    }

    function test_BatchAnchor() public {
        bytes32[] memory ids = new bytes32[](2);
        bytes32[] memory hashes = new bytes32[](2);
        uint8[] memory kinds = new uint8[](2);
        (ids[0], ids[1]) = (RECORD, keccak256("claim:2"));
        (hashes[0], hashes[1]) = (HASH_A, HASH_B);
        (kinds[0], kinds[1]) = (1, 1);
        ledger.anchorBatch(ids, hashes, kinds);
        _eq(ledger.totalEntries(), 2, "batch total");
        uint8[] memory short = new uint8[](1);
        vm.expectRevert(ClaimLedger.LengthMismatch.selector);
        ledger.anchorBatch(ids, hashes, short);
    }

    function test_TwoStepOwnershipTransfer() public {
        ledger.transferOwnership(STRANGER);
        require(ledger.owner() == address(this), "not yet transferred");
        vm.expectRevert(ClaimLedger.NotPendingOwner.selector);
        ledger.acceptOwnership();
        vm.prank(STRANGER);
        ledger.acceptOwnership();
        require(ledger.owner() == STRANGER, "transferred");
    }

    function testFuzz_AnyNonZeroHashAnchorsOnce(bytes32 h) public {
        if (h == bytes32(0)) return;
        ledger.anchor(RECORD, h, 2, "");
        require(ledger.isAnchored(h), "anchored");
        vm.expectRevert(abi.encodeWithSelector(ClaimLedger.AlreadyAnchored.selector, h));
        ledger.anchor(RECORD, h, 2, "");
    }
}
