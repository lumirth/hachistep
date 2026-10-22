// Cartridge I/O only. Position independent ARMv5TE; all protocol logic stays in HGSS.
.syntax unified
.arch armv5te
.arm
.text
start:
    push {r4}
    adr r2, start
    add r12, r2, #144
    add r2, r2, #484
    ldr r1, [r2, #20]
    cmp r1, #0
    ldrne r1, [r2]
    cmp r1, #0
    moveq r1, #0
    strbeq r1, [r0]
    moveq r0, #0
    popeq {r4}
    bxeq lr
    ldr r1, [r2, #4]
    mov r3, r0
    mov r0, r1
1:  ldrb r4, [r12], #1
    eor r4, r4, #0xaa
    strb r4, [r3], #1
    subs r1, r1, #1
    bne 1b
    str r1, [r2]
    pop {r4}
    bx lr
.org 144
rx: .space 185
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
    cmp r1, #185
    bhi fault
    cmp r1, #0
    beq done
    ldr r3, [r2, #8]
    cmp r3, #0
    bne fault
    mov r3, r1
2:  ldrb r4, [r0], #1
    strb r4, [r12], #1
    subs r1, r1, #1
    bne 2b
    str r3, [r2, #8]
    b done
.org 448
done:
    pop {r4, r5}
    bx lr
fault:
    mov r3, #1
    str r3, [r2, #16]
    b done
.org 484
mailbox: .space 28
epoch: .space 16
.org 532
identity: .ascii "HachiStep IR v2\0"
.org 564
status:
    mov r0, #0
    bx lr
.org 572
tx: .space 185
.org 948
