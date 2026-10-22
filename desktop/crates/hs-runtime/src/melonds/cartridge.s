// ARMv5TE cartridge adapter. Rebuild with python3 tools/melonds_cartridge.py.
.syntax unified
.arch armv5te
.arm
.text
start:
    push {r4, r5}
    adr r2, start
    add r12, r2, #144
    add r3, r2, #128
    add r2, r2, #484
    ldr r1, [r2, #20]
    cmp r1, #0
    beq empty_return
    ldr r5, [r3, #4]
    ldr r1, [r2]
    cmp r1, r5
    beq upkeep
    ldr r1, [r3, #8]
    mov r3, r0
    mov r0, r1
1:  ldrb r4, [r12], #1
    eor r4, r4, #0xaa
    strb r4, [r3], #1
    subs r1, r1, #1
    bne 1b
    str r5, [r2]
    pop {r4, r5}
    bx lr
empty_return:
    mov r1, #0
    strb r1, [r0]
    mov r0, #0
    pop {r4, r5}
    bx lr
.org 128
// Host owns TX consumed, RX published, RX length, and heartbeat sequence.
// The RX bytes and maintenance frame follow.
host: .space 16
rx: .space 136
maintenance: .space 8
.org 332
write:
    adr r2, write
    sub r2, r2, #332
    add r12, r2, #572
    add r2, r2, #484
    ldr r3, [r2, #20]
    cmp r3, #0
    bxeq lr
    push {r4, r5}
    sub r3, r2, #356
    cmp r1, #136
    bhi fault
    cmp r1, #0
    beq done
    ldr r4, [r2, #4]
    ldr r5, [r3]
    sub r5, r4, r5
    cmp r5, #2
    bhs fault
    tst r4, #1
    addne r12, r12, #136
    mov r3, r1
2:  ldrb r5, [r0], #1
    strb r5, [r12], #1
    subs r1, r1, #1
    bne 2b
    tst r4, #1
    streq r3, [r2, #8]
    strne r3, [r2, #12]
    add r4, r4, #1
    str r4, [r2, #4]
done:
    pop {r4, r5}
    bx lr
fault:
    mov r3, #1
    str r3, [r2, #16]
    b done
.org 484
// Guest publishes RX consumed, TX published, two TX lengths, and fault.
// The host controls active. The guest updates the maintenance countdown.
mailbox: .space 28
epoch: .space 16
.org 532
identity: .ascii "HachiStep IR v4\0"
.org 548
last_host: .word 0
.org 564
status:
    mov r0, #0
    bx lr
.org 572
tx: .space 272
.org 844
// HGSS polls every 4 ms. Stop maintenance after 255 empty receive calls
// without a host publication, about one guest second.
upkeep:
    ldr r1, [r2, #64]
    ldr r5, [r2, #-344]
    cmp r1, r5
    strne r5, [r2, #64]
    ldr r1, [r2, #24]
    movne r1, #255
    subs r1, r1, #1
    str r1, [r2, #24]
    ble empty_return
    ldr r4, [r2, #-204]
    and r1, r4, #255
    cmp r1, #0x2e
    bne empty_return
    str r4, [r0]
    ldr r4, [r2, #-200]
    str r4, [r0, #4]
    mov r0, #8
    pop {r4, r5}
    bx lr
.org 948
