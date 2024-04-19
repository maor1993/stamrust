MEMORY
{
  RAM    (xrw)    : ORIGIN = 0x20000000,   LENGTH = 24K
  RAM2    (rw)    : ORIGIN = 0x20006000,   LENGTH = 16K
  FLASH    (rx)    : ORIGIN = 0x8000000,   LENGTH = 128K
}

SECTIONS {
     .ram2bss (NOLOAD) : ALIGN(4) {
       *(.ram2bss);
       . = ALIGN(4);
     } > RAM2
   } ;