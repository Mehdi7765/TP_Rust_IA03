MEMORY
{
  FLASH  : ORIGIN = 0x08000000, LENGTH = 2M

  /* AXISRAM */
  RAM : ORIGIN = 0x24000000, LENGTH = 512K

  /* DTCM  */
  DTCMRAM    : ORIGIN = 0x20000000, LENGTH = 128K

  /* SRAM */
  SRAM1 : ORIGIN = 0x30000000, LENGTH = 128K
  SRAM2 : ORIGIN = 0x30020000, LENGTH = 128K
  SRAM3 : ORIGIN = 0x30040000, LENGTH = 32K
  SRAM4 : ORIGIN = 0x38000000, LENGTH = 64K

  /* Backup SRAM */
  BSRAM : ORIGIN = 0x38800000, LENGTH = 4K

  /* Instruction TCM */
  ITCM  : ORIGIN = 0x00000000, LENGTH = 64K
}


/* Pile (stack) dans la DTCM (128 Ko, inutilisee sinon) : toute la RAM AXI de 512 Ko
   reste pour les variables statiques et le tas de Burn (480 Ko). */
_stack_start = ORIGIN(DTCMRAM) + LENGTH(DTCMRAM);
_stack_end = ORIGIN(DTCMRAM);

