I was annoyed by the BIOS fan control on my laptop, so decided to try and re-implement it. The code works, and implements a PID controller [https://en.wikipedia.org/wiki/PID_controller], poking the ACPI Embedded Controller to control the CPU and GPU fans.

Most of the work involved was in experimentation with the ACPI Embedded Controller.  

You can dump out the ACPI tables and examine the code etc, which sort of doesn't give you direct information, but was still pretty useful, with snippets like:

```
ReadRegister 0xb2 RPM1 8, RPM2 8
WriteRegister 0x53 CTYP Cooling type
```
,
```
                Offset (0x40),
                SW2S,   1,
                    ,   2,
                ACCC,   1,
                TRPM,   1,
```
and
```
            Method (FRSP, 0, NotSerialized)
            {
                Local2 = Zero
                If ((\_SB.PCI0.LPCB.EC0.ECOK == One))
                {
                    Local0 = \_SB.PCI0.LPCB.EC0.RPM1
                    Local1 = \_SB.PCI0.LPCB.EC0.RPM2
                    Local1 <<= 0x08
                    Local0 |= Local1
                    If ((Local0 != Zero))
                    {
                        Divide (0x00075300, Local0, Local0, Local2)
                    }
                }

                Return (Local2)
            }

            Method (FSSP, 1, NotSerialized)
            {
                If ((\_SB.PCI0.LPCB.EC0.ECOK == One))
                {
                    If ((Arg0 != Zero))
                    {
                        \_SB.PCI0.LPCB.EC0.SFAN = Zero
                    }
                    Else
                    {
                        \_SB.PCI0.LPCB.EC0.SFAN = 0x02
                    }
                }
            }
```

Then using the utility `ec-probe` to dump out the Embedded Controller registers and experiment with writing values to them, I managed to determine the fans needed first to be put in to a "manual" mode before I could start to set the fan speeds directly.

The x-axis shows the values in the ACPI EC registers changing over time.

```
0x45: 92                           ,90                                 ,92                                 ,90
0x47: 36,35            ,34                        ,33                        ,32                     ,31
0x48: 3F         ,3E      ,3D         ,3C                                    ,3B                  ,3A         ,39
0x50: 00            ,20,00      ,20,00   ,20,00               ,20,00               ,20,00      ,20,00   ,20,00
0x58: 52,54,51   ,4C,46   ,40   ,39   ,38,3E   ,4F      ,50                     ,4B   ,45,3E   ,38   ,36,35
0x59: 39   ,3A,3B,3A   ,39   ,38         ,37,38   ,39,3A,3B      ,3C,3D            ,3C,3B   ,3A,39   ,38   ,37
0x87: DC,DA,DB   ,DC,DB,DA   ,DB      ,DA,DB,DA,DB   ,DC,DB   ,DA,DB,DC,DA,DB,DA,DC,DB,DA,DB                  ,DA
0x8F: 00   ,01,02,00,01,02,00   ,01,02,00   ,02,00      ,01,02   ,00,01,02,00,01,02,00   ,02,00         ,01,02,00
0x93: 1C,14      ,1C,14   ,1C   ,14   ,1C   ,14,1C   ,14                  ,1C,14   ,1C   ,14,1C      ,14
0x94: 28,2A   ,C4,EB   ,FF,EF               ,C8,C4,8C                  ,9C,8C   ,EB,EF   ,FF,EF
0x9D: DC         ,DB                                          ,DA
0xB2: CD,E1,D4   ,EF,DA,39,B4,B6,98,7C,60,68,61,18,B6,AE,B6,BF,B2   ,B6,0C,AC,C6,DA,CD,E0,39,C3,42,94   ,8E,83,63
0xB3: 0E               ,0D,0C,0B,0A,09,07      ,09,0B                  ,0C,0E         ,0D   ,0B   ,0A         ,07
0xB7: 37   ,36,35,34,33   ,32   ,31         ,32,33,34                     ,33   ,32,31   ,30   ,2F
0xC3: 0F   ,15,1B,0F   ,83,F6,E9,CE,00         ,E9,F7,EC,F0,EC,F0,F3,EC,20,E6,15   ,09,29,83,EC,83,C5   ,B9,CB,00
0xC4: 0E               ,0C,0B,0A,09,00         ,08,0A                  ,0B,0D,0E      ,0D,0C,0A   ,09         ,00
```

nb. Readme has been pieced together from memory!
