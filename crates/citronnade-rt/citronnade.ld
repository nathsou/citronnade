/*
 * Memory layout of a 3DSX program.
 *
 * The 3DSX format has three segments that must be contiguous and start on
 * page boundaries: code (read/execute), read-only data, and data (read/write,
 * with .bss at its end, which the program clears itself). The entry point is
 * the start of the code segment. Relocations are kept in the ELF
 * (--emit-relocs) so the 3DSX writer can build its relocation tables.
 */
OUTPUT_FORMAT("elf32-littlearm")
OUTPUT_ARCH(arm)
ENTRY(_start)

PHDRS
{
    code   PT_LOAD FLAGS(5);
    rodata PT_LOAD FLAGS(4);
    data   PT_LOAD FLAGS(6);
}

SECTIONS
{
    . = 0x00100000;

    .text : ALIGN(4)
    {
        KEEP(*(.crt0))
        *(.text .text.*)
        *(.glue_7 .glue_7t .vfp11_veneer .v4_bx)
        . = ALIGN(4);
    } :code

    . = ALIGN(0x1000);

    .rodata : ALIGN(4)
    {
        *(.rodata .rodata.*)
        . = ALIGN(4);
    } :rodata

    .ARM.extab : ALIGN(4) { *(.ARM.extab*) . = ALIGN(4); } :rodata

    .ARM.exidx : ALIGN(4)
    {
        __exidx_start = .;
        *(.ARM.exidx*)
        __exidx_end = .;
        . = ALIGN(4);
    } :rodata

    . = ALIGN(0x1000);

    .data : ALIGN(4)
    {
        *(.data .data.*)
        . = ALIGN(4);
    } :data

    .init_array : ALIGN(4)
    {
        __init_array_start = .;
        KEEP(*(SORT(.init_array.*)))
        KEEP(*(.init_array))
        __init_array_end = .;
    } :data

    .bss (NOLOAD) : ALIGN(8)
    {
        __bss_start = .;
        *(.bss .bss.* COMMON)
        . = ALIGN(8);
        __bss_end = .;
    } :data

    __end = .;

    /DISCARD/ : { *(.ARM.attributes*) *(.comment) *(.note*) }
}
