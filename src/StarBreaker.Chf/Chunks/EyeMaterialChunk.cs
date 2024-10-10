﻿
using StarBreaker.Common;

namespace StarBreaker.Chf;

public sealed class EyeMaterialChunk
{
    public const uint Key = 0xA047885E;
    
    public required ColorsChunk EyeColors { get; init; }
    
    public static EyeMaterialChunk Read(ref SpanReader reader)
    {
        if (reader.Peek<uint>() != Key)
        {
            return new EyeMaterialChunk
            {
                EyeColors = new ColorsChunk()
                {
                    Color01 = new Color(),
                    Color02 = new Color(),
                }
            };
        }

        reader.Expect(Key);
        reader.Expect(Guid.Empty);
        reader.ExpectAny([0xCE9DF055, 0xD5354502]);
        reader.Expect(Guid.Empty);
        reader.Expect(1);
        reader.Expect(5);
        reader.ExpectAny([0x9736C44B,0x8C9E711C]);
        reader.Expect<uint>(0);
        reader.Expect<uint>(0);
        reader.Expect<uint>(0);
        var colorBlock = ColorsChunk.Read(ref reader);
        //TODO: why
        if (reader.Remaining >= 4)
            reader.Expect<uint>(5);
        
        return new EyeMaterialChunk
        {
            EyeColors = colorBlock
        };
    }
}