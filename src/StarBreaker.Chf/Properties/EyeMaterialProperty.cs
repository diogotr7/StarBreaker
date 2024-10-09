﻿
using StarBreaker.Common;

namespace StarBreaker.Chf;

public sealed class EyeMaterialProperty
{
    public const uint Key = 0xA047885E;
    
    public required ColorsProperty EyeColors { get; init; }
    
    public static EyeMaterialProperty Read(ref SpanReader reader)
    {
        if (reader.Peek<uint>() != Key)
        {
            return new EyeMaterialProperty
            {
                EyeColors = new ColorsProperty()
                {
                    Color01 = new Color(),
                    Color02 = new Color(),
                }
            };
        }

        reader.Expect(Key);
        reader.Expect(Guid.Empty);
        reader.Expect(0xCE9DF055);
        reader.Expect(Guid.Empty);
        reader.Expect(1);
        reader.Expect(5);
        reader.Expect(0x9736C44B);
        reader.Expect<uint>(0);
        reader.Expect<uint>(0);
        reader.Expect<uint>(0);
        var colorBlock = ColorsProperty.Read(ref reader);
        reader.Expect<uint>(5);
        
        return new EyeMaterialProperty
        {
            EyeColors = colorBlock
        };
    }
}